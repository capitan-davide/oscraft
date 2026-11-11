//! `random`: OsCraft's random-number device, a native resource manager
//! serving `/dev/random` and `/dev/urandom` (one path, with `-p`, for a
//! test). A ChaCha20 generator, keyed from what the system has of
//! entropy at start: the cycle counter sampled around sleeps (timing
//! jitter), the time of day, the process ID, and what is written to the
//! device, which is mixed into the key. Reads never block: as FreeBSD's
//! `/dev/random` once seeded, and as QNX's `random` service.
//!
//! The framework is QNX's (`resmgr`): a dispatch handle, one
//! attachment per path with the POSIX defaults, and a read function of
//! its own.

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};
use core::ptr::{null, null_mut};
use std::sync::Mutex;

use kercall::abi::Iov;
use kercall::neutrino::msg::MsgReply;
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_NOREPLY, ResmgrAttr};
use resmgr::dispatch::{dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler};
use resmgr::iofunc::{iofunc_attr_init, iofunc_func_init, iofunc_read_verify, iofunc_write_verify};
use resmgr::resmgr::resmgr_attach;

const S_IFCHR: u32 = 0o020000;

/// ChaCha20 (RFC 8439): a 64-byte block from the key, the counter and a
/// nonce; the generator's stream.
struct ChaCha {
    state: [u32; 16],
}

impl ChaCha {
    fn new(key: &[u8; 32], nonce: &[u8; 12]) -> ChaCha {
        let mut state = [0u32; 16];
        state[..4].copy_from_slice(&[0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574]);
        for i in 0..8 {
            state[4 + i] = u32::from_le_bytes(key[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 0..3 {
            state[13 + i] = u32::from_le_bytes(nonce[i * 4..i * 4 + 4].try_into().unwrap());
        }
        ChaCha { state }
    }

    fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(16);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(12);
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(8);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(7);
    }

    /// The next block of the stream.
    fn block(&mut self) -> [u8; 64] {
        let mut s = self.state;
        for _ in 0..10 {
            Self::quarter(&mut s, 0, 4, 8, 12);
            Self::quarter(&mut s, 1, 5, 9, 13);
            Self::quarter(&mut s, 2, 6, 10, 14);
            Self::quarter(&mut s, 3, 7, 11, 15);
            Self::quarter(&mut s, 0, 5, 10, 15);
            Self::quarter(&mut s, 1, 6, 11, 12);
            Self::quarter(&mut s, 2, 7, 8, 13);
            Self::quarter(&mut s, 3, 4, 9, 14);
        }
        let mut out = [0u8; 64];
        for i in 0..16 {
            out[i * 4..i * 4 + 4].copy_from_slice(&s[i].wrapping_add(self.state[i]).to_le_bytes());
        }
        self.state[12] = self.state[12].wrapping_add(1);
        if self.state[12] == 0 {
            self.state[13] = self.state[13].wrapping_add(1);
        }
        out
    }

    /// Rekeys from the stream's own next block mixed with `extra`
    /// (what a writer gave, or a fresh jitter sample): forward secrecy
    /// for what was given out before.
    fn rekey(&mut self, extra: &[u8]) {
        let block = self.block();
        let mut key = [0u8; 32];
        key.copy_from_slice(&block[..32]);
        for (i, b) in extra.iter().enumerate() {
            key[i % 32] ^= *b;
        }
        let mut nonce = [0u8; 12];
        nonce.copy_from_slice(&block[32..44]);
        *self = ChaCha::new(&key, &nonce);
    }
}

fn cycles() -> u64 {
    kercall::arch::clock_cycles()
}

/// A key from the timing jitter of short sleeps, the clock and the pid:
/// what this system has of entropy without a hardware source (a
/// board's RNG device, once its startup publishes it, goes here).
fn seed() -> ([u8; 32], [u8; 12]) {
    let mut acc = [0u8; 44];
    let mut h: u64 = 0x9e37_79b9_7f4a_7c15 ^ cycles();
    let now = kercall::time::_syspage_time(0);
    h = h.wrapping_mul(0x100_0000_01b3) ^ now;
    h = h.wrapping_mul(0x100_0000_01b3) ^ (std::process::id() as u64);
    for i in 0..352 {
        let before = cycles();
        std::thread::sleep(std::time::Duration::from_micros(50 + (h % 97)));
        let delta = cycles().wrapping_sub(before);
        h = (h ^ delta).wrapping_mul(0x100_0000_01b3).rotate_left(23);
        acc[i % 44] ^= (h >> ((i % 8) * 8)) as u8;
    }
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 12];
    key.copy_from_slice(&acc[..32]);
    nonce.copy_from_slice(&acc[32..]);
    (key, nonce)
}

static GEN: Mutex<Option<ChaCha>> = Mutex::new(None);

/// `_IO_READ`: `nbytes` of the stream, never blocking.
unsafe extern "C" fn io_read(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: the framework's functions with its own arguments.
    let e = unsafe { iofunc_read_verify(ctp, msg, ocb.cast::<IofuncOcb>(), null_mut()) };
    if e != 0 {
        return e;
    }
    // SAFETY: `_io_read`'s nbytes is at 4.
    let want = unsafe { msg.add(4).cast::<u32>().read() } as usize;
    let mut buf = vec![0u8; want.min(1 << 16)];
    {
        let mut g = GEN.lock().unwrap();
        let rng = g.as_mut().expect("seeded");
        let mut done = 0;
        while done < buf.len() {
            let block = rng.block();
            let n = (buf.len() - done).min(64);
            buf[done..done + n].copy_from_slice(&block[..n]);
            done += n;
        }
    }
    // SAFETY: the context is the framework's.
    let rcvid = unsafe { (*ctp).rcvid };
    MsgReply(rcvid, buf.len() as c_long, buf.as_ptr().cast(), buf.len());
    RESMGR_NOREPLY
}

/// `_IO_WRITE`: what is written is mixed into the key.
unsafe extern "C" fn io_write(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: the framework's functions with its own arguments.
    let e = unsafe { iofunc_write_verify(ctp, msg, ocb.cast::<IofuncOcb>(), null_mut()) };
    if e != 0 {
        return e;
    }
    // SAFETY: `_io_write`'s nbytes is at 4; the data follows the 16-byte header in the receive buffer.
    let n = unsafe { msg.add(4).cast::<u32>().read() } as usize;
    // SAFETY: the context's buffer holds at least what fits in it.
    let avail = unsafe { (*ctp).msg_max_size }.saturating_sub(16).min(n);
    // SAFETY: as above.
    let data = unsafe { core::slice::from_raw_parts(msg.add(16), avail) };
    GEN.lock().unwrap().as_mut().expect("seeded").rekey(data);
    // SAFETY: the context is the framework's.
    unsafe { (*ctp).status = n as c_long };
    0
}

static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };
static mut ATTRS: [IofuncAttr; 2] = unsafe { core::mem::zeroed() };

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut paths: Vec<String> = vec!["/dev/random".into(), "/dev/urandom".into()];
    if let Some(i) = args.iter().position(|a| a == "-p") {
        paths = vec![args.get(i + 1).cloned().unwrap_or_default()];
    }
    let (key, nonce) = seed();
    *GEN.lock().unwrap() = Some(ChaCha::new(&key, &nonce));
    let cpaths: Vec<Vec<u8>> = paths.iter().map(|p| { let mut v = p.clone().into_bytes(); v.push(0); v }).collect();
    // SAFETY: the framework's functions, with statics of this program.
    unsafe {
        // ANCHOR: serve
        let dpp = dispatch_create();
        if dpp.is_null() {
            eprintln!("random: dispatch_create failed");
            std::process::exit(1);
        }
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut IO)).funcs[0] = Some(io_read);
        (*(&raw mut IO)).funcs[1] = Some(io_write);
        let attr = ResmgrAttr { flags: 0, nparts_max: 1, msg_max_size: 4096, other_func: None, reserved: [0; 4] };
        for (i, p) in cpaths.iter().enumerate().take(2) {
            let a = (&raw mut ATTRS).cast::<IofuncAttr>().add(i);
            iofunc_attr_init(a, S_IFCHR | 0o666, null(), null());
            let id = resmgr_attach(dpp, &attr, p.as_ptr().cast::<c_char>(), 0, 0, &raw const CONNECT, &raw const IO, a.cast::<c_void>());
            if id < 0 {
                eprintln!("random: resmgr_attach({}) failed, errno {}", paths[i], std::io::Error::last_os_error());
                std::process::exit(1);
            }
        }
        let ctp = dispatch_context_alloc(dpp);
        if ctp.is_null() {
            eprintln!("random: dispatch_context_alloc failed");
            std::process::exit(1);
        }
        loop {
            if dispatch_block(ctp).is_null() {
                eprintln!("random: dispatch_block failed: {}", std::io::Error::last_os_error());
                std::process::exit(1);
            }
            dispatch_handler(ctp);
        }
        // ANCHOR_END: serve
    }
    #[allow(unreachable_code)]
    {
        let _ = (c_uint::MAX, Iov::raw(null(), 0));
    }
}

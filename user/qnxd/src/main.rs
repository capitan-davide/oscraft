//! `qnxd`: the QNX personality's service, opt-in, under `/compat/qnx`. The kernel
//! hands it each message a QNX program sends to a native server (`ker::msg`,
//! plain reads and writes aside). A thread of the pool takes the message,
//! sends it on with `MsgForwardv()`, which is `MsgSendv()` with the program
//! as the sender the server sees, blocks for the answer as any client, and
//! replies to the program.
//!
//! Native servers use FreeBSD's `struct stat` and `struct termios`, which
//! it converts. Everything else goes through as it came, as they still speak QNX's
//! layouts there.

use core::ffi::{c_int, c_long, c_void};
use core::ptr::null;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use kercall::neutrino::channel::ChannelCreate;
use kercall::neutrino::msg::{
    MsgError, MsgForwardv, MsgRead, MsgReceive, MsgReply,
};
use resmgr::resmgr::pathmgr_attach;
use qnxabi::fcntl::{flags_from_qnx, flags_to_qnx, ioflag_from_qnx};
use proto::connect::{CONNECT_COMBINE, CONNECT_COMBINE_CLOSE};
use proto::info::MsgInfo;
use proto::stat::{Stat, FORM_FREEBSD, FORM_PREFERRED};
use proto::termios::{Termios, QnxTermios};
use qnxabi::termios::{from_qnx, to_qnx};

unsafe extern "C" {
    fn pipe(fds: *mut c_int) -> c_int;
    fn fork() -> c_int;
    fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
}

/// What a message shows of itself, the path of an open included, and
/// the room for a server's answer.
const HEAD: usize = 512;
const REPLY: usize = size_of::<Stat>();
/// The largest message or reply it carries.
const MAX: isize = 1 << 20;
const IO_CONNECT: u16 = 0x100;
const IO_STAT: u16 = 0x104;
const IO_DEVCTL: u16 = 0x106;
/// `struct _io_devctl`, and its reply.
const DEVCTL: usize = 16;
const IO_OPENFD: u16 = 0x10d;
/// `_IO_SYNC`, in `proto::io`.
const IO_SYNC: u16 = proto::io::IO_SYNC;
/// `DCMD_ALL_GETFLAGS`, `_SETFLAGS`.
const GETFLAGS: u32 = 0x0101;
const SETFLAGS: u32 = 0x0102;
const TIOCGETA: u32 = 0x7413;
const TIOCSETA: u32 = 0x7414;
const TIOCSETAF: u32 = 0x7416;
/// `struct _io_connect` up to its path; its extra data follows the path,
/// aligned to 8.
const CONNECT_SIZE: usize = 40;

/// How this service makes itself known, as QNX's dumper with
/// `_FTYPE_DUMPER`: a file type of its own, in the path manager.
const FTYPE_QNX: u32 = 0x4f53;
const RESMGR_FLAG_FTYPEONLY: u32 = 0x8;

/// `_NTO_CHF_DISCONNECT`-free: nobody connects to this channel.
/// `_NTO_CHF_UNBLOCK`: a signal must not take a client out of a message
/// the server may already be acting on, as it would not on the server
/// itself (a `close()` lost to `SIGCHLD`). The kernel passes the request
/// on to the server (`ker::msg::unblock`).
const IO_READ: u16 = 0x101;
const CHANNEL_FLAGS: u32 = 0x2;

fn main() {
    // The child serves; the parent waits until it is registered, so that
    // the boot script goes on with QNX's programs served.
    let mut ready = [0 as c_int; 2];
    // SAFETY: no other threads; the array has room for two.
    if unsafe { pipe(ready.as_mut_ptr()) } != 0 {
        std::process::exit(1);
    }
    // SAFETY: as above.
    let child = unsafe { fork() };
    if child > 0 {
        let mut byte = 0u8;
        // SAFETY: a byte on this stack.
        let n = unsafe { read(ready[0], (&raw mut byte).cast(), 1) };
        std::process::exit(if n == 1 { 0 } else { 1 });
    }
    let chid = ChannelCreate(CHANNEL_FLAGS);
    if child < 0 || chid < 0 || unsafe { pathmgr_attach(null(), chid, 0, FTYPE_QNX, RESMGR_FLAG_FTYPEONLY, false) }.is_err() {
        eprintln!("qnxd: can't register: {}", std::io::Error::last_os_error());
        std::process::exit(1);
    }
    // SAFETY: a byte in static memory.
    unsafe { write(ready[1], b"r".as_ptr().cast(), 1) };
    let idle = Arc::new(AtomicUsize::new(0));
    worker(chid, idle);
}

/// A thread of the pool: when it takes a message and no other is
/// waiting, it starts another, so that a slow server holds up one client.
fn worker(chid: c_int, idle: Arc<AtomicUsize>) -> ! {
    let mut head = [0u8; HEAD];
    let mut request = Vec::new();
    let mut reply = Vec::new();
    loop {
        let mut info = MsgInfo::default();
        idle.fetch_add(1, Ordering::SeqCst);
        let rcvid = MsgReceive(chid, head.as_mut_ptr().cast(), HEAD, (&raw mut info).cast());
        if idle.fetch_sub(1, Ordering::SeqCst) == 1 {
            let (more, chid) = (idle.clone(), chid);
            let _ = std::thread::Builder::new().spawn(move || worker(chid, more));
        }
        if rcvid <= 0 {
            continue;
        }
        forward(rcvid, &head, &info, &mut request, &mut reply);
    }
}

/// One client's message, to its server and back.
fn forward(rcvid: i64, head: &[u8; HEAD], info: &MsgInfo, request: &mut Vec<u8>, reply: &mut Vec<u8>) {
    let (len, room) = (info.srcmsglen.clamp(0, MAX), info.dstmsglen.clamp(0, MAX).max(REPLY as isize));
    request.resize(len as usize, 0);
    reply.resize(room as usize, 0);
    if MsgRead(rcvid, request.as_mut_ptr().cast(), len as usize, 0) < 0 {
        MsgError(rcvid, kercall::tls::get_errno());
        return;
    }
    // The flags, FreeBSD's for QNX's, in what carries them.
    let u32_of = |m: &[u8], at: usize| u32::from_le_bytes([m[at], m[at + 1], m[at + 2], m[at + 3]]);
    let ty = u16::from_le_bytes([request[0], request[1]]);
    match ty {
        IO_CONNECT if request.len() >= CONNECT_SIZE => {
            let flag = ioflag_from_qnx(u32_of(request, 20));
            request[20..24].copy_from_slice(&flag.to_le_bytes());
        }
        IO_OPENFD if request.len() >= 8 => {
            let flag = ioflag_from_qnx(u32_of(request, 4));
            request[4..8].copy_from_slice(&flag.to_le_bytes());
        }
        IO_SYNC if request.len() >= 8 => {
            let flag = flags_from_qnx(u32_of(request, 4));
            request[4..8].copy_from_slice(&flag.to_le_bytes());
        }
        _ => {}
    }
    if devctl_dcmd(request) == Some(SETFLAGS) && request.len() >= DEVCTL + 4 {
        let flag = flags_from_qnx(u32_of(request, DEVCTL));
        request[DEVCTL..DEVCTL + 4].copy_from_slice(&flag.to_le_bytes());
    }
    // `tcsetattr()`: FreeBSD's settings for QNX's, in place, and the
    // command says the size.
    let dcmd = devctl_dcmd(request);
    let mut len = len;
    if matches!(dcmd, Some(TIOCSETA..=TIOCSETAF)) && request.len() >= DEVCTL + 76 {
        // SAFETY: the length is checked; any bytes are QNX's settings.
        let q = unsafe { request.as_ptr().add(DEVCTL).cast::<QnxTermios>().read_unaligned() };
        let bsd = from_qnx(&q);
        request[4..8].copy_from_slice(&(0x8000_0000u32 | 44 << 16 | dcmd.unwrap_or(0)).to_le_bytes());
        request[8..12].copy_from_slice(&44u32.to_le_bytes());
        // SAFETY: as above.
        unsafe { request.as_mut_ptr().add(DEVCTL).cast::<Termios>().write_unaligned(bsd) };
        len = (DEVCTL + 44) as isize;
    } else if dcmd == Some(TIOCGETA) {
        request[4..8].copy_from_slice(&(0x4000_0000u32 | 44 << 16 | TIOCGETA).to_le_bytes());
        request[8..12].copy_from_slice(&44u32.to_le_bytes());
    }
    let mut replied = 0u64;
    let status = MsgForwardv(
        rcvid,
        request.as_ptr().cast(),
        -(len as i64),
        reply.as_mut_ptr().cast(),
        -(room as i64),
        &raw mut replied,
    );
    if status < 0 {
        MsgError(rcvid, kercall::tls::get_errno());
    } else if dcmd == Some(TIOCGETA) && replied as usize >= DEVCTL + 44 {
        // `tcgetattr()`: QNX's settings for FreeBSD's. QNX's extra
        // characters aren't kept: none has a value on a native terminal.
        // SAFETY: the length is checked; any bytes are FreeBSD's settings.
        let t = unsafe { reply.as_ptr().add(DEVCTL).cast::<Termios>().read_unaligned() };
        let q = to_qnx(&t, &QnxTermios::default());
        let mut out = [0u8; DEVCTL + 76];
        out[..DEVCTL].copy_from_slice(&reply[..DEVCTL]);
        out[8..12].copy_from_slice(&76u32.to_le_bytes());
        // SAFETY: the array holds them.
        unsafe { out.as_mut_ptr().add(DEVCTL).cast::<QnxTermios>().write_unaligned(q) };
        MsgReply(rcvid, status as c_long, out.as_ptr().cast(), out.len());
    } else if dcmd == Some(GETFLAGS) && replied as usize >= DEVCTL + 4 {
        let flag = flags_to_qnx(u32_of(reply, DEVCTL));
        reply[DEVCTL..DEVCTL + 4].copy_from_slice(&flag.to_le_bytes());
        MsgReply(rcvid, status as c_long, reply.as_ptr().cast(), replied as usize);
    } else if is_readdir(head) {
        // FreeBSD's records for QNX's, which are never longer.
        let mut out = vec![0u8; replied as usize];
        let n = qnxabi::dirent::to_qnx(&reply[..replied as usize], &mut out);
        MsgReply(rcvid, n as c_long, out.as_ptr().cast(), n);
    } else if wants_stat(head) && status as u32 == FORM_FREEBSD {
        // SAFETY: a reply of that form is FreeBSD's `struct stat`.
        let st = unsafe { reply.as_ptr().cast::<Stat>().read_unaligned() };
        let st = qnxabi::stat::to_qnx(&st, qnxabi::stat::EXTRA);
        MsgReply(rcvid, c_long::from(FORM_PREFERRED as i32), (&raw const st).cast(), size_of_val(&st));
    } else {
        MsgReply(rcvid, status as c_long, reply.as_ptr().cast(), replied as usize);
    }
}

/// The low half of a device command's number, if the message is one.
fn devctl_dcmd(msg: &[u8]) -> Option<u32> {
    (msg.len() >= DEVCTL && msg[..2] == IO_DEVCTL.to_le_bytes()).then(|| u32::from(msg[4]) | u32::from(msg[5]) << 8)
}

/// Whether the message reads a directory (`_IO_XTYPE_READDIR`).
fn is_readdir(msg: &[u8; HEAD]) -> bool {
    u16::from_le_bytes([msg[0], msg[1]]) == IO_READ && msg[8] == 9
}

/// Whether the message is a `stat()`: `_IO_STAT`, alone or combined with
/// an open, so that its reply is FreeBSD's `struct stat`.
fn wants_stat(msg: &[u8; HEAD]) -> bool {
    let u16_at = |at: usize| u16::from_le_bytes([msg[at], msg[at + 1]]);
    if u16_at(0) == IO_STAT {
        return true;
    }
    // `struct _io_connect` with an extra message after its path.
    let (extra_len, path_len) = (u16_at(38), usize::from(u16_at(34)));
    let extra = (CONNECT_SIZE + path_len + 7) & !7;
    u16_at(0) == IO_CONNECT
        && matches!(u16_at(2), CONNECT_COMBINE | CONNECT_COMBINE_CLOSE)
        && extra_len != 0
        && extra + 2 <= HEAD
        && u16_at(extra) == IO_STAT
}

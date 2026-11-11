//! procnto's text console (QNX `pathmgr/devtext.c`): a resource manager on
//! channel `DEVTEXT_CHID` that writes to the kernel console, attached as
//! `/dev/text`, `/dev/console` and `/dev/tty` (see `pathmgr`). New
//! processes get it as stdin, stdout and stderr. It keeps no per-open
//! state, so opening, closing and `dup()` need no work.
//!
//! I/O messages (`<sys/iomsg.h>`): `_IO_WRITE` (0x102) is `struct
//! _io_write` { type, combine_len, nbytes, xtype, zero } followed by the
//! data; the reply status is the number of bytes written. `_IO_DEVCTL`
//! answers `DCMD_CHR_ISATTY` (it is a terminal) and the open-flags
//! commands, ENOTTY otherwise.
//! `_IO_STAT` describes a character device.
//!
//! The open flags of each open are kept with procnto's other open-file
//! contexts (`resmgr::text_bind`); the descriptors procnto gives a new
//! process have none and are read/write. As procnto's: a write on a
//! descriptor not open for writing is EBADF, a read that would wait on a
//! non-blocking one EAGAIN; `lseek()`, `chmod()`, `chown()` and the like
//! are ENOSYS, `fsync()` (from ENOSYS) and `fpathconf()` EINVAL, `fstatvfs()` ENOSYS.
//! Unlike procnto's, it is a terminal (`tcgetattr()` works): OsCraft reads
//! the console through it.
//!
//! Input: the debug callouts can only be polled, so a second thread
//! (`poller`) polls the console every 10 ms, queues what arrives and
//! pulses the server. It polls only while there is someone to read the
//! input: a held read, or a foreground process group (for the signal
//! keys); otherwise it sleeps until there is (no timer: a tickless CPU
//! stays idle), and the console's device is left alone, so that a driver
//! (e.g. QNX's `devc-serpl011`, when the console is a serial port) can have
//! it. `_IO_READ` requests are held (not replied to) until
//! there is input, then get what is there, as a QNX resource manager
//! would do. Input goes through the line discipline in `tty`, which the
//! terminal devctls (`tcgetattr()`, `tcsetattr()`, ...) configure.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::kercall::call;
use crate::ker::errno::{EAGAIN, EBADF, EBADMSG, EINVAL, ENOSYS, ENOTTY};
use crate::ker::event::SIGEV_SIZE;
use crate::ker::number::*;
use crate::ker::process::Personality;
use crate::ker::process::{DEVTEXT_CHID, KERNEL, KERNEL_PID, SIDE_CHANNEL};
use crate::sync::IrqSpinLock;
use crate::{kprintln, tprint};

use super::resmgr::Client;
use super::tty::Tty;
use crate::personality::qnx;
use super::{pathmgr, resmgr};

const IO_READ: u16 = 0x101;
const IO_WRITE: u16 = 0x102;
const IO_STAT: u16 = 0x104;
const IO_DEVCTL: u16 = 0x106;
const IO_PATHCONF: u16 = 0x108;
const IO_OPENFD: u16 = 0x10d;
const IO_DUP: u16 = 0x115;
const IO_CLOSE: u16 = 0x116;
/// `struct _io_dup`: the original connection's pid and ID, in its `info`.
const DUP_PID: usize = 12;
const DUP_COID: usize = 28;
/// `_IO_FLAG_WR`, `O_NONBLOCK`.
const IO_FLAG_WR: u32 = 0x2;
const O_NONBLOCK: u32 = 0x4;
/// The open flags of a descriptor procnto made (`_IO_FLAG_RD | _IO_FLAG_WR`).
const DEFAULT_IOFLAG: u32 = 0x3;
const IO_WRITE_HDR: usize = 16;
/// `_IO_NOTIFY`: { type, combine_len, action, flags, zero, event, ... },
/// and the reply's conditions at 4.
const IO_NOTIFY: u16 = 0x105;
const NOTIFY_FLAGS: usize = 8;
const NOTIFY_EVENT: usize = 16;
const NOTIFY_REPLY: usize = size_of::<u64>();
/// `_NOTIFY_ACTION_POLLARM` and `_NOTIFY_COND_INPUT`, `_OUTPUT`.
const NOTIFY_ACTION_POLLARM: i32 = 3;
const NOTIFY_COND_INPUT: u32 = 0x1000_0000;
const NOTIFY_COND_OUTPUT: u32 = 0x2000_0000;
/// `__DIOT(_DCMD_CHR, 8, pid_t)`.
const DCMD_CHR_TCSETSID: u32 = 0x8004_0308;
// _IO_CONNECT subtypes
const CONNECT_COMBINE: u16 = 0;
const CONNECT_COMBINE_CLOSE: u16 = 1;
const CONNECT_OPEN: u16 = 2;
const S_IFCHR: u32 = 0o020000;
// devctl commands (<sys/dcmd_all.h>, <sys/dcmd_chr.h>): __DIOF(_DCMD_ALL,
// 1, int), __DIOT(_DCMD_ALL, 2, int), __DION(_DCMD_CHR, 24).
const DCMD_ALL_GETFLAGS: i32 = 0x4004_0101;
const DCMD_ALL_SETFLAGS: i32 = 0x8004_0102u32 as i32;
const DCMD_CHR_ISATTY: i32 = (3 << 8) + 24;
const DCMD_FSYS_STATVFS: i32 = 0x40a8_020d;
// Terminal devctls (<sys/ioctl.h>, values from the SDP's compiler).
// The low half: FreeBSD's and QNX's differ in the struct's size.
const TIOCGETA: i32 = 0x7413;
const TIOCSETA: i32 = 0x7414;
const TIOCSETAF: i32 = 0x7416;
const TIOCGWINSZ: i32 = 0x4008_7468;
const TIOCSWINSZ: i32 = 0x8008_7467u32 as i32;
const TIOCGPGRP: i32 = 0x4004_7477;
const TIOCSPGRP: i32 = 0x8004_7476u32 as i32;
/// `struct _io_devctl` { type, combine_len, dcmd, nbytes, zero }, then the
/// data; the reply is `struct _io_devctl_reply` { zero, ret_val, nbytes,
/// zero2 } and the data.
const DEVCTL_HDR: usize = 16;

/// Bytes typed on the console, not read yet.
static INPUT: IrqSpinLock<VecDeque<u8>> = IrqSpinLock::new(VecDeque::new());
/// Pulse code from the poller: input arrived.
const PULSE_INPUT: i32 = 1;
const POLL_NS: u64 = 10_000_000;
/// Whether the console has a reader (see the module documentation).
static WANTED: AtomicBool = AtomicBool::new(false);
/// The poller's thread id, for waking it when the console gets a reader.
static POLLER: AtomicU32 = AtomicU32::new(0);

// ANCHOR: devtext_poller
/// Poll the console for input every 10 ms (the debug callouts have no
/// interrupt) and tell the server when some arrived.
pub fn poller(_: usize) {
    let coid = call(
        __KER_CONNECT_ATTACH,
        [
            0,
            KERNEL_PID as u64,
            DEVTEXT_CHID as u64,
            SIDE_CHANNEL as u64,
            0,
            0,
        ],
    )
    .expect("devtext connection");
    let me = crate::sched::current();
    POLLER.store(me.tid, Ordering::Release);
    loop {
        if !WANTED.load(Ordering::Acquire) {
            // Until `update_wanted` wakes it; a reader that came meanwhile
            // is seen again after marking itself blocked.
            crate::sched::prepare_block(crate::sched::State::Nanosleep);
            if WANTED.load(Ordering::Acquire) {
                crate::sched::wake(me);
            }
            crate::sched::block();
            continue;
        }
        let mut got = false;
        while WANTED.load(Ordering::Acquire)
            && let Some(c) = crate::console::poll_key()
        {
            // Bounded as the line discipline's input: while the server
            // is behind, more is dropped, not queued (review PROCMGR-16).
            let mut input = INPUT.lock();
            if input.len() < super::tty::INPUT_MAX {
                input.push_back(c);
            }
            got = true;
        }
        if got {
            let _ = call(
                __KER_MSG_SEND_PULSE,
                [coid, (-1i64) as u64, PULSE_INPUT as u64, 0, 0, 0],
            );
        }
        crate::timeout::sleep_until(crate::clock::now_ns() + POLL_NS);
    }
}
// ANCHOR_END: devtext_poller

/// The server's state: the terminal and the reads waiting for input.
struct State {
    tty: Tty,
    /// (rcvid, nbytes) of the held reads, oldest first.
    pending: VecDeque<(u64, usize)>,
    /// Those waiting for input in `poll()`: who, its message, its event.
    armed: Vec<(Client, u64, [u8; SIGEV_SIZE])>,
}

impl State {
    /// Pass the bytes that arrived through the line discipline, then reply
    /// to the reads that can now be satisfied, oldest first.
    fn serve_reads(&mut self) {
        loop {
            let Some(b) = INPUT.lock().pop_front() else {
                break;
            };
            if let Some(sig) = self.tty.input(b)
                && self.tty.pgrp != 0
            {
                crate::ker::signal::post_group(self.tty.pgrp, sig);
            }
        }
        while let Some(&(rcvid, nbytes)) = self.pending.front() {
            let Some(data) = self.tty.read(nbytes) else {
                break;
            };
            self.pending.pop_front();
            // The reader may have left (a signal): keep its input.
            if !try_reply(rcvid, data.len() as u64, &data) {
                self.tty.unread(data);
            }
        }
        if self.tty.readable() {
            for (_, rcvid, event) in self.armed.drain(..) {
                let _ = call(__KER_MSG_DELIVER_EVENT, [rcvid, event.as_ptr() as u64, 0, 0, 0, 0]);
            }
        }
        self.update_wanted();
    }

    /// Poll the console while someone reads it or may type a signal key.
    fn update_wanted(&self) {
        let pgrp = self.tty.pgrp;
        let foreground = pgrp != 0
            && crate::ker::process::all()
                .iter()
                .any(|p| p.pgid() == pgrp && !p.is_zombie());
        let wanted = !self.pending.is_empty() || !self.armed.is_empty() || foreground;
        if wanted && !WANTED.swap(true, Ordering::AcqRel) {
            if let Some(t) = crate::sched::thread(POLLER.load(Ordering::Acquire)) {
                crate::sched::wake(&t);
            }
        } else if !wanted {
            WANTED.store(false, Ordering::Release);
        }
    }
}

fn reply(rcvid: u64, status: u64, data: &[u8]) {
    let _ = try_reply(rcvid, status, data);
}

/// Reply; false if the client is no longer waiting.
fn try_reply(rcvid: u64, status: u64, data: &[u8]) -> bool {
    call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            status,
            data.as_ptr() as u64,
            (-(data.len() as i64)) as u64,
            0,
            0,
        ],
    )
    .is_ok()
}

fn error(rcvid: u64, errno: u32) {
    let _ = call(__KER_MSG_ERROR, [rcvid, errno as u64, 0, 0, 0, 0]);
}

/// Whether a client is a FreeBSD process.
fn native(pid: i32) -> bool {
    crate::ker::process::lookup(pid).is_some_and(|p| p.personality() == Personality::Native)
}

/// `_IO_NOTIFY`: the conditions asked for that hold, input when a read
/// would return and output always; with none and `_NOTIFY_ACTION_POLLARM`,
/// the event is kept for when input arrives. Any other action disarms.
fn notify(rcvid: u64, client: Client, msg: &[u8], st: &mut State) {
    let action = i32::from_le_bytes(msg[4..8].try_into().unwrap());
    let flags = u32::from_le_bytes(msg[NOTIFY_FLAGS..NOTIFY_FLAGS + 4].try_into().unwrap());
    st.armed.retain(|(c, ..)| *c != client);
    st.serve_reads();
    let input = if st.tty.readable() { NOTIFY_COND_INPUT } else { 0 };
    let held = flags & (input | NOTIFY_COND_OUTPUT);
    if held == 0 && action == NOTIFY_ACTION_POLLARM {
        let mut event = [0; SIGEV_SIZE];
        event.copy_from_slice(&msg[NOTIFY_EVENT..NOTIFY_EVENT + SIGEV_SIZE]);
        st.armed.push((client, rcvid, event));
    }
    st.update_wanted();
    let mut r = [0u8; 32];
    r[4..NOTIFY_REPLY].copy_from_slice(&held.to_le_bytes());
    reply(rcvid, 0, &r);
}

/// `_IO_DEVCTL`: `struct _io_devctl` { type, combine_len, dcmd, nbytes,
/// zero }; the reply is `struct _io_devctl_reply` { zero, ret_val, nbytes,
/// zero2 } followed by the data.
fn devctl(rcvid: u64, client: Client, msg: &[u8], st: &mut State) {
    let pid = client.pid;
    let dcmd = i32::from_le_bytes(msg[4..8].try_into().unwrap());
    let data = msg.get(DEVCTL_HDR..).unwrap_or(&[]);
    let int = |v: i32| v.to_le_bytes();
    let ioflag = resmgr::text_ioflag(client).unwrap_or(DEFAULT_IOFLAG);
    let native = resmgr::native(pid);
    let mut out_termios = Vec::new();
    let out: Option<&[u8]> = match dcmd {
        DCMD_CHR_ISATTY | TIOCSWINSZ => None,
        DCMD_ALL_GETFLAGS => Some(&int(resmgr::getfl_for(pid, ioflag) as i32)),
        // `fcntl(F_SETFL)`: answered, the flags kept, as procnto's
        // `/dev/text` does (measured, issue #291).
        DCMD_ALL_SETFLAGS if data.len() >= 4 => None,
        DCMD_FSYS_STATVFS => return error(rcvid, ENOSYS),
        d if d & 0xffff == TIOCGETA => {
            let t = st.tty.get_attr();
            if native {
                out_termios.extend_from_slice(&t);
            } else {
                out_termios.extend_from_slice(&qnx::termios_out(&t, &st.tty.qnx_base));
            }
            Some(&out_termios)
        }
        d if (TIOCSETA..=TIOCSETAF).contains(&(d & 0xffff)) => {
            let flush = d & 0xffff == TIOCSETAF;
            if native {
                st.tty.set_attr(data, flush);
            } else if let Some(q) = data.first_chunk::<76>() {
                st.tty.qnx_base = *q;
                st.tty.set_attr(&qnx::termios_in(q), flush);
            }
            None
        }
        // struct winsize { ws_row, ws_col, ws_xpixel, ws_ypixel }
        TIOCGWINSZ => Some(&[24, 0, 80, 0, 0, 0, 0, 0]),
        // Until there are sessions, a terminal without a foreground process
        // group reports the caller's (every process is its own group).
        TIOCGPGRP if st.tty.pgrp == 0 => Some(&int(pid)),
        TIOCGPGRP => Some(&int(st.tty.pgrp)),
        TIOCSPGRP if data.len() >= 4 => {
            st.tty.pgrp = i32::from_le_bytes(data[..4].try_into().unwrap());
            None
        }
        // The boot script's `[+session]`, through `session::set_terminal`:
        // the console's session, its leader's group in the foreground.
        // From process 1 alone: to a program it stays ENOTTY.
        d if d as u32 == DCMD_CHR_TCSETSID && pid == KERNEL_PID && data.len() >= 4 => {
            st.tty.pgrp = i32::from_le_bytes(data[..4].try_into().unwrap());
            None
        }
        // Other commands: ENOTTY, as procnto's managers answer.
        _ => return error(rcvid, ENOTTY),
    };
    let out = out.unwrap_or(&[]);
    let mut r = Vec::with_capacity(DEVCTL_HDR + out.len());
    r.extend_from_slice(&[0; 8]); // zero, ret_val
    r.extend_from_slice(&(out.len() as u32).to_le_bytes());
    r.extend_from_slice(&[0; 4]);
    r.extend_from_slice(out);
    reply(rcvid, 0, &r);
    // A settings change can make held reads satisfiable (or not).
    st.serve_reads();
}

/// An I/O message, received directly or combined with an `_IO_CONNECT`.
fn io(rcvid: u64, client: Client, msg: &[u8], st: &mut State) {
    let pid = client.pid;
    let ty = u16::from_le_bytes([msg[0], msg[1]]);
    if let Err(e) = resmgr::io_check(ty, msg.len()) {
        return error(rcvid, e);
    }
    let ioflag = resmgr::text_ioflag(client).unwrap_or(DEFAULT_IOFLAG);
    match ty {
        IO_READ if msg.len() >= 16 => {
            // Held until there is input (a zero-byte read returns at once);
            // on a non-blocking descriptor, what there is or EAGAIN.
            let nbytes = u32::from_le_bytes(msg[4..8].try_into().unwrap()) as usize;
            // pread() (an xtype): ENOSYS, as procnto's.
            if msg[8] != 0 {
                error(rcvid, ENOSYS);
            } else if nbytes == 0 {
                reply(rcvid, 0, &[]);
            } else if ioflag & O_NONBLOCK != 0 {
                st.serve_reads();
                match st.pending.is_empty().then(|| st.tty.read(nbytes)).flatten() {
                    Some(data) => reply(rcvid, data.len() as u64, &data),
                    None => error(rcvid, EAGAIN),
                }
            } else {
                st.pending.push_back((rcvid, nbytes));
                st.serve_reads();
            }
        }
        IO_WRITE if ioflag & IO_FLAG_WR == 0 => error(rcvid, EBADF),
        IO_WRITE if msg.len() >= IO_WRITE_HDR => {
            let nbytes = u32::from_le_bytes(msg[4..8].try_into().unwrap()) as usize;
            let data = &msg[IO_WRITE_HDR..msg.len().min(IO_WRITE_HDR + nbytes)];
            tprint!(
                "{}",
                core::str::from_utf8(data).unwrap_or("<non-UTF-8 output>")
            );
            reply(rcvid, data.len() as u64, &[]);
        }
        IO_DEVCTL if msg.len() >= DEVCTL_HDR => devctl(rcvid, client, msg, st),
        // `poll()` of a FreeBSD program. A QNX one gets procnto's ENOSYS,
        // below.
        IO_NOTIFY if native(pid) && msg.len() >= NOTIFY_EVENT + SIGEV_SIZE => notify(rcvid, client, msg, st),
        IO_STAT => {
            // A character device, read/write for its owner, of size 1, as
            // procnto's.
            let (form, st) = resmgr::dev_stat(1, S_IFCHR | 0o600, 1, resmgr::DEV_TEXT_RDEV).encode(client.pid);
            reply(rcvid, form as u64, &st);
        }
        resmgr::IO_FDINFO => {
            let (status, r) = resmgr::fdinfo(msg, S_IFCHR | 0o600, ioflag, 0, 0, "/dev/text");
            reply(rcvid, status as u64, &r);
        }
        IO_CLOSE => {
            resmgr::text_bind(client, None);
            reply(rcvid, 0, &[]);
        }
        // dup(): of a descriptor of this device (the client's own or one
        // passed on), with its open flags; another one's is EBADF.
        IO_DUP => {
            let u32_at = |o: usize| u32::from_le_bytes(msg[o..o + 4].try_into().unwrap());
            let from = Client {
                pid: match u32_at(DUP_PID) as i32 {
                    0 => pid,
                    p => p,
                },
                coid: u32_at(DUP_COID) as i32,
            };
            let ours = KERNEL.channel(DEVTEXT_CHID);
            let theirs = crate::ker::process::lookup(from.pid).and_then(|p| p.connection(from.coid));
            match (ours, theirs) {
                (Some(ch), Some(c)) if alloc::sync::Arc::ptr_eq(&ch, &c.channel) => {
                    if let Some(f) = resmgr::text_ioflag(from) {
                        resmgr::text_bind(client, Some(f));
                    }
                    reply(rcvid, 0, &[]);
                }
                _ => error(rcvid, EBADF),
            }
        }
        IO_PATHCONF => error(rcvid, EINVAL),
        // openfd(): EBADF for no access mode, else ENOSYS (procnto's).
        IO_OPENFD if msg[4] & 3 == 0 => error(rcvid, EBADF),
        // `_IO_SYNC` (libc makes it EINVAL), `_IO_LSEEK`, `_IO_CHMOD`,
        // `_IO_CHOWN`, `_IO_UTIME`, `_IO_LOCK`, `_IO_READ64`, `_IO_WRITE64`,
        // `_IO_SPACE`, `_IO_NOTIFY` and the other types procnto's managers
        // don't handle.
        0x103 | 0x105 | 0x107 | 0x109..=0x10c | 0x10f..=0x111 | 0x113 | 0x114 | 0x117 | 0x118 | 0x11a..=0x11d
        | IO_OPENFD | 0x119 | 0x11e..=0x121 => error(rcvid, ENOSYS),
        _ => {
            kprintln!(
                "[pid {pid}] devtext: I/O message {ty:#06x} ({} bytes) not implemented",
                msg.len()
            );
            error(rcvid, ENOSYS);
        }
    }
}

/// `_IO_CONNECT` for one of the device's names (see `pathmgr`): the open
/// is bound to the client's connection, with its flags.
fn connect(rcvid: u64, client: Client, msg: &[u8], st: &mut State) {
    let Some(c) = pathmgr::parse(msg, client.pid) else {
        return error(rcvid, EINVAL);
    };
    // Not over a context the connection still has (`resmgr::unbound`): an
    // open is EBUSY; a combined close (a stat()) leaves it be.
    let free = resmgr::unbound(client);
    match (c.subtype, free) {
        (CONNECT_OPEN | CONNECT_COMBINE, Err(e)) => return error(rcvid, e),
        (CONNECT_OPEN | CONNECT_COMBINE | CONNECT_COMBINE_CLOSE, Ok(())) => resmgr::text_bind(client, Some(c.ioflag)),
        (CONNECT_COMBINE_CLOSE, Err(_)) => {}
        _ => return error(rcvid, ENOSYS),
    }
    if c.subtype != CONNECT_OPEN && c.combined.len() >= 4 {
        io(rcvid, client, c.combined, st);
    } else {
        reply(rcvid, 0, &[]);
    }
    if c.subtype == CONNECT_COMBINE_CLOSE && free.is_ok() {
        resmgr::text_bind(client, None);
    }
}

pub fn server(_: usize) {
    let mut st = State {
        tty: Tty::new(),
        pending: VecDeque::new(), armed: Vec::new(),
    };
    let mut msg = [0u8; 2048];
    let mut info = [0u8; 64];
    loop {
        let Ok(rcvid) = call(
            __KER_MSG_RECEIVEV,
            [
                DEVTEXT_CHID as u64,
                msg.as_mut_ptr() as u64,
                (-(msg.len() as i64)) as u64,
                info.as_mut_ptr() as u64,
                0,
                0,
            ],
        ) else {
            continue;
        };
        if rcvid == 0 {
            st.serve_reads(); // input arrived
            continue;
        }
        let client = Client {
            pid: i32::from_le_bytes(info[8..12].try_into().unwrap()),
            coid: i32::from_le_bytes(info[24..28].try_into().unwrap()),
        };
        let len = (i64::from_le_bytes(info[32..40].try_into().unwrap()) as usize).min(msg.len());
        if len < 2 {
            error(rcvid, EBADMSG);
        } else if u16::from_le_bytes([msg[0], msg[1]]) == pathmgr::IO_CONNECT {
            connect(rcvid, client, &msg[..len], &mut st);
        } else {
            io(rcvid, client, &msg[..len], &mut st);
        }
        st.update_wanted();
    }
}

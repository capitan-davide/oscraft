//! `pty`: OsCraft's pseudo-terminals, a native resource manager in place
//! of QNX's `devc-pty`, with its names and defaults: `-n` pairs (8, at
//! most 256), `/dev/pty[p-zP-T][0-9a-f]` the controller and
//! `/dev/tty[p-zP-T][0-9a-f]` the worker.
//!
//! As `devc-pty` (`pty_master_open`, `pty_slave_open`, `pty_devctl`): a
//! second open of a controller is EBUSY; the worker's first open sets
//! the terminal's defaults; `grantpt()`'s devctl gives the worker to the
//! caller, mode 0620. The rest is the line discipline QNX's `io-char`
//! library provides: canonical input with editing and echo, the signal
//! keys, ONLCR on output, window size and process groups, on FreeBSD's
//! `struct termios`, which `qnxd` lays out for QNX's programs. FreeBSD's
//! `posix_openpt()`, `ptsname()` and `grantpt()` come through
//! `TIOCPTMASTER` and `FIODGNAME`, answered here.
//!
//! QNX's `ptsname()` takes the controller's path from `iofdinfo()` and
//! wants `isatty()` false on it, so a controller refuses `DCMD_CHR_ISATTY`.
//!
//! Differs from `devc-pty` so far: a blocking open of a worker without a
//! controller is EIO instead of waiting for one; `TCSETSID` records the session but
//! does not tell the process manager (`/dev/tty` doesn't reach a pty);
//! no flow control, no background-process job control.

use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::{null, null_mut};
use std::collections::VecDeque;

use kercall::neutrino::msg::{MsgError, MsgRead, MsgReply};
use kercall::neutrino::signal::SignalKill;
use proto::errno as e;
use proto::io::{Devctl, DevctlReply};
use resmgr::abi::{
    ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, Notify, RESMGR_DEFAULT, RESMGR_NOREPLY,
    ResmgrAttr,
};
use resmgr::dispatch::{
    dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler,
};
use resmgr::iofunc::{
    iofunc_attr_init, iofunc_client_info, iofunc_close_ocb_default, iofunc_devctl_default,
    iofunc_func_init, iofunc_notify, iofunc_notify_trigger, iofunc_open_default,
    iofunc_read_verify, iofunc_write_verify,
};
use resmgr::resmgr::resmgr_attach;

const S_IFCHR: u32 = 0o020000;
const O_NONBLOCK: i32 = 0x4;
const IO_FLAG_MASK: u32 = 3;

// The I/O functions' indexes (the message type from `_IO_READ`).
const IO_READ: usize = 0x00;
const IO_WRITE: usize = 0x01;
const IO_CLOSE_OCB: usize = 0x02;
const IO_NOTIFY: usize = 0x04;
const IO_DEVCTL: usize = 0x05;
const IO_UNBLOCK: usize = 0x06;

// FreeBSD's termios bits.
const IGNCR: u32 = 0x80;
const INLCR: u32 = 0x40;
const ICRNL: u32 = 0x100;
const OPOST: u32 = 1;
const ONLCR: u32 = 2;
const ISIG: u32 = 0x80;
const ICANON: u32 = 0x100;
const ECHO: u32 = 8;
const ECHOE: u32 = 2;
const ECHONL: u32 = 0x10;
const NOFLSH: u32 = 0x8000_0000;
const IEXTEN: u32 = 0x400;
const VEOF: usize = 0;
const VEOL: usize = 1;
const VERASE: usize = 3;
const VWERASE: usize = 4;
const VKILL: usize = 5;
const VINTR: usize = 8;
const VQUIT: usize = 9;
const VSUSP: usize = 10;
const VMIN: usize = 16;
const NCCS: usize = 20;
const TERMIOS_SIZE: usize = 44;
// FreeBSD's signal numbers, the kernel's.
const SIGHUP: c_int = 1;
const SIGINT: c_int = 2;
const SIGQUIT: c_int = 3;
const SIGTSTP: c_int = 18;
const SIGWINCH: c_int = 28;

/// io-char's buffers (`kernel/src/proc/tty.rs`), and the room for what
/// the worker wrote until the controller reads it (`-O`).
const CANON_MAX: usize = 256;
const INPUT_MAX: usize = 2048;
const OUTPUT_MAX: usize = 4096;

/// A read held until there is something to read.
struct Reader {
    rcvid: i64,
    nbytes: usize,
}

/// A worker's write held until the controller makes room.
struct Writer {
    rcvid: i64,
    data: Vec<u8>,
    done: usize,
}

/// One pseudo-terminal.
struct Pty {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    cc: [u8; NCCS],
    ispeed: u32,
    ospeed: u32,
    /// The line being edited; input ready for the worker; the length of
    /// the complete lines in it; what the worker wrote, for the controller.
    line: Vec<u8>,
    ready: VecDeque<u8>,
    lines: VecDeque<usize>,
    output: VecDeque<u8>,
    worker_reads: Vec<Reader>,
    controller_reads: Vec<Reader>,
    worker_writes: VecDeque<Writer>,
    winsize: [u8; 8],
    pgrp: i32,
    sid: i32,
    /// Its controller or worker has been closed since the last open.
    controller_open: bool,
    worker_open: bool,
    /// Worker's and controller's armed notifications.
    notify: [[Notify; 3]; 2],
    index: usize,
}

/// A device: the attribute first, so that an OCB's `attr` finds this.
#[repr(C)]
struct Dev {
    attr: IofuncAttr,
    pty: *mut Pty,
    controller: bool,
}

impl Pty {
    fn new(index: usize) -> Pty {
        let mut cc = [0u8; NCCS];
        cc[VINTR] = 3;
        cc[VQUIT] = 0x1c;
        cc[VERASE] = 0x7f;
        cc[VKILL] = 0x15;
        cc[VEOF] = 4;
        cc[VSUSP] = 0x1a;
        cc[VWERASE] = 0x17;
        cc[VMIN] = 1;
        let mut p = Pty {
            iflag: 0,
            oflag: 0,
            cflag: 0,
            lflag: 0,
            cc,
            ispeed: 38400,
            ospeed: 38400,
            line: Vec::new(),
            ready: VecDeque::new(),
            lines: VecDeque::new(),
            output: VecDeque::new(),
            worker_reads: Vec::new(),
            controller_reads: Vec::new(),
            worker_writes: VecDeque::new(),
            winsize: [0; 8],
            pgrp: 0,
            sid: 0,
            controller_open: false,
            worker_open: false,
            // SAFETY: an empty list, as `iofunc_notify_init()` makes.
            notify: unsafe { core::mem::zeroed() },
            index,
        };
        p.defaults();
        p
    }

    /// `pty_slave_open`'s settings, for the worker's first open.
    fn defaults(&mut self) {
        self.iflag = 0x2b02;
        self.oflag = 3;
        self.cflag = 0x4b00;
        self.lflag = 0x5cb;
    }

    fn canonical(&self) -> bool {
        self.lflag & ICANON != 0
    }

    fn is(&self, b: u8, c: usize) -> bool {
        self.cc[c] != 0 && self.cc[c] == b
    }

    /// Output for the controller, as the terminal's settings have it.
    fn put(&mut self, b: u8) {
        if b == b'\n' && self.oflag & OPOST != 0 && self.oflag & ONLCR != 0 {
            self.output.push_back(b'\r');
        }
        self.output.push_back(b);
    }

    fn echo(&mut self, b: u8) {
        if self.lflag & ECHO == 0 {
            return;
        }
        if b < 0x20 && b != b'\n' && b != b'\t' {
            self.output.push_back(b'^');
            self.output.push_back(b + 0x40);
        } else {
            self.put(b);
        }
    }

    fn erase(&mut self) -> bool {
        if self.line.pop().is_none() {
            return false;
        }
        if self.lflag & ECHO != 0 && self.lflag & ECHOE != 0 {
            self.output.extend(*b"\x08 \x08");
        }
        true
    }

    fn end_line(&mut self) {
        self.lines.push_back(self.line.len());
        let line = core::mem::take(&mut self.line);
        self.ready.extend(line);
    }

    /// A byte from the controller, which types on this terminal. The
    /// signal it raises, if any.
    fn input(&mut self, mut b: u8) -> Option<c_int> {
        if self.lflag & ISIG != 0 {
            for (c, s) in [(VINTR, SIGINT), (VQUIT, SIGQUIT), (VSUSP, SIGTSTP)] {
                if self.is(b, c) {
                    self.echo(b);
                    if self.lflag & NOFLSH == 0 {
                        self.line.clear();
                        self.ready.clear();
                        self.lines.clear();
                    }
                    return Some(s);
                }
            }
        }
        if b == b'\r' {
            if self.iflag & IGNCR != 0 {
                return None;
            }
            if self.iflag & ICRNL != 0 {
                b = b'\n';
            }
        } else if b == b'\n' && self.iflag & INLCR != 0 {
            b = b'\r';
        }
        if !self.canonical() {
            if self.ready.len() < INPUT_MAX {
                self.echo(b);
                self.ready.push_back(b);
            }
            return None;
        }
        if self.is(b, VERASE) || b == 8 {
            self.erase();
        } else if self.is(b, VKILL) {
            while self.erase() {}
        } else if self.lflag & IEXTEN != 0 && self.is(b, VWERASE) {
            while self.line.last() == Some(&b' ') && self.erase() {}
            while self.line.last().is_some_and(|&c| c != b' ') && self.erase() {}
        } else if self.is(b, VEOF) {
            self.end_line();
        } else if b == b'\n' || self.is(b, VEOL) {
            if self.ready.len() + self.line.len() < INPUT_MAX {
                if self.lflag & ECHO != 0 || self.lflag & ECHONL != 0 {
                    self.put(b'\n');
                }
                self.line.push(b);
                self.end_line();
            }
        } else if self.line.len() + 2 <= CANON_MAX
            && self.ready.len() + self.line.len() + 2 <= INPUT_MAX
        {
            self.echo(b);
            self.line.push(b);
        }
        None
    }

    /// What a worker's read takes now, if it needn't wait.
    fn take_input(&mut self, nbytes: usize) -> Option<Vec<u8>> {
        if self.canonical() {
            let line = self.lines.front_mut()?;
            let n = nbytes.min(*line);
            *line -= n;
            if *line == 0 {
                self.lines.pop_front();
            }
            return Some(self.ready.drain(..n).collect());
        }
        if self.ready.is_empty() && self.cc[VMIN] > 0 {
            return None;
        }
        let n = nbytes.min(self.ready.len());
        Some(self.ready.drain(..n).collect())
    }

    /// What the worker can read at once.
    fn input_count(&self) -> usize {
        if self.canonical() {
            self.lines.iter().sum::<usize>()
                + self.lines.len().min(1) * usize::from(self.ready.is_empty())
        } else {
            self.ready.len()
        }
    }

    /// Everything that can move now does: held reads answered, held
    /// writes taken, notifications fired.
    fn settle(&mut self) {
        loop {
            while self.output.len() < OUTPUT_MAX {
                let Some(w) = self.worker_writes.front_mut() else {
                    break;
                };
                let room = OUTPUT_MAX - self.output.len();
                let n = room.min(w.data.len() - w.done);
                let chunk = w.data[w.done..w.done + n].to_vec();
                w.done += n;
                if w.done == w.data.len() {
                    let w = self.worker_writes.pop_front().unwrap();
                    MsgReply(w.rcvid, w.done as c_long, null(), 0);
                }
                for b in chunk {
                    self.put(b);
                }
            }
            let mut moved = false;
            while !self.output.is_empty() && !self.controller_reads.is_empty() {
                let r = self.controller_reads.remove(0);
                let n = r.nbytes.min(self.output.len());
                let data: Vec<u8> = self.output.drain(..n).collect();
                MsgReply(
                    r.rcvid,
                    data.len() as c_long,
                    data.as_ptr().cast(),
                    data.len(),
                );
                moved = true;
            }
            while !self.worker_reads.is_empty() {
                let nbytes = self.worker_reads[0].nbytes;
                let Some(data) = self.take_input(nbytes) else {
                    break;
                };
                let r = self.worker_reads.remove(0);
                MsgReply(
                    r.rcvid,
                    data.len() as c_long,
                    data.as_ptr().cast(),
                    data.len(),
                );
            }
            if !moved {
                break;
            }
        }
        if !self.controller_open {
            for r in self.worker_reads.drain(..) {
                MsgReply(r.rcvid, 0, null(), 0);
            }
            for w in self.worker_writes.drain(..) {
                MsgError(w.rcvid, e::EIO);
            }
        }
        if !self.worker_open {
            for r in self.controller_reads.drain(..) {
                MsgReply(r.rcvid, 0, null(), 0);
            }
        }
        let worker_in = self.input_count() as c_int + c_int::from(!self.controller_open);
        let room = (OUTPUT_MAX - self.output.len().min(OUTPUT_MAX)) as c_int;
        let controller_in = self.output.len() as c_int + c_int::from(!self.worker_open);
        // SAFETY: the entries of this terminal.
        unsafe {
            if worker_in > 0 {
                iofunc_notify_trigger(self.notify[0].as_mut_ptr(), worker_in, 0);
            }
            if room > 0 {
                iofunc_notify_trigger(self.notify[0].as_mut_ptr(), room, 1);
            }
            if controller_in > 0 {
                iofunc_notify_trigger(self.notify[1].as_mut_ptr(), controller_in, 0);
            }
            iofunc_notify_trigger(self.notify[1].as_mut_ptr(), 1, 1);
        }
    }

    fn signal(&self, sig: c_int) {
        if self.pgrp > 0 {
            SignalKill(0, -self.pgrp, 0, sig, 0, 0);
        }
    }

    /// `/dev/` less: the worker's name.
    fn name(&self) -> String {
        name("tty", self.index)
    }
}

/// `pty[p-zP-T][0-9a-f]` and `tty...`, as `devc-pty`.
fn name(prefix: &str, index: usize) -> String {
    const LETTERS: &[u8] = b"pqrstuvwxyzPQRST";
    format!("{prefix}{}{:x}", LETTERS[index / 16] as char, index % 16)
}

/// The device of an OCB.
unsafe fn dev(ocb: *mut c_void) -> &'static mut Dev {
    // SAFETY: the OCB's attribute is a `Dev`'s, its first field.
    unsafe { &mut *(*ocb.cast::<IofuncOcb>()).attr.cast::<Dev>() }
}

unsafe fn rcvid(ctp: *mut Context) -> i64 {
    // SAFETY: the context is the framework's.
    unsafe { (*ctp).rcvid }
}

unsafe extern "C" fn io_open(
    ctp: *mut Context,
    msg: *mut u8,
    handle: *mut c_void,
    extra: *mut c_void,
) -> c_int {
    let d = handle.cast::<Dev>();
    // SAFETY: the attribute handed to `resmgr_attach()`.
    let (controller, p) = unsafe { ((*d).controller, &mut *(*d).pty) };
    // SAFETY: `_io_connect`'s ioflag, in the message.
    let ioflag = unsafe {
        msg.cast::<proto::connect::Connect>()
            .read_unaligned()
            .ioflag
    };
    let access = ioflag & IO_FLAG_MASK != 0;
    if controller {
        if access && p.controller_open {
            return e::EBUSY;
        }
    } else if !p.controller_open && ioflag as i32 & O_NONBLOCK == 0 {
        return e::EIO;
    }
    // SAFETY: the framework's default with its own arguments.
    let r = unsafe { iofunc_open_default(ctp, msg, handle.cast(), extra) };
    if r == 0 && access {
        if controller {
            p.controller_open = true;
        } else {
            if !p.worker_open {
                p.defaults();
                p.pgrp = 0;
                p.sid = 0;
            }
            p.worker_open = true;
        }
    }
    r
}

unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this manager.
    let d = unsafe { dev(ocb) };
    // A `stat()`'s open has no access, and doesn't count as the end's.
    // SAFETY: as above.
    let access = unsafe { (*ocb.cast::<IofuncOcb>()).ioflag } as u32 & IO_FLAG_MASK != 0;
    // SAFETY: the framework's default with its own arguments.
    let r = unsafe { iofunc_close_ocb_default(ctp, msg, ocb.cast()) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    if access && d.attr.rcount == 0 && d.attr.wcount == 0 {
        if d.controller {
            p.controller_open = false;
            p.signal(SIGHUP);
            p.output.clear();
        } else {
            p.worker_open = false;
            p.line.clear();
            p.ready.clear();
            p.lines.clear();
        }
        p.settle();
    }
    r
}

unsafe extern "C" fn io_read(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let mut nonblock = 0;
    // SAFETY: the framework's function with its own arguments.
    let r = unsafe { iofunc_read_verify(ctp, msg, ocb.cast::<IofuncOcb>(), &raw mut nonblock) };
    if r != 0 {
        return r;
    }
    // SAFETY: `_io_read`'s nbytes is at 4.
    let nbytes = unsafe { msg.add(4).cast::<u32>().read_unaligned() } as usize;
    // SAFETY: an OCB of this manager, and the context's message.
    let (d, id) = unsafe { (dev(ocb), rcvid(ctp)) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    let data = if nbytes == 0 {
        Some(Vec::new())
    } else if d.controller {
        if !p.output.is_empty() {
            let n = nbytes.min(p.output.len());
            Some(p.output.drain(..n).collect())
        } else if !p.worker_open {
            Some(Vec::new())
        } else {
            None
        }
    } else {
        p.take_input(nbytes).or(if p.controller_open {
            None
        } else {
            Some(Vec::new())
        })
    };
    let Some(data) = data else {
        if nonblock != 0 {
            return e::EAGAIN;
        }
        let r = Reader { rcvid: id, nbytes };
        if d.controller {
            p.controller_reads.push(r);
        } else {
            p.worker_reads.push(r);
        }
        return RESMGR_NOREPLY;
    };
    MsgReply(id, data.len() as c_long, data.as_ptr().cast(), data.len());
    p.settle();
    RESMGR_NOREPLY
}

unsafe extern "C" fn io_write(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let mut nonblock = 0;
    // SAFETY: the framework's function with its own arguments.
    let r = unsafe { iofunc_write_verify(ctp, msg, ocb.cast::<IofuncOcb>(), &raw mut nonblock) };
    if r != 0 {
        return r;
    }
    // SAFETY: `_io_write`'s nbytes is at 4.
    let n = unsafe { msg.add(4).cast::<u32>().read_unaligned() } as usize;
    // SAFETY: an OCB of this manager, and the context's message.
    let (d, id) = unsafe { (dev(ocb), rcvid(ctp)) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    let mut data = vec![0u8; n];
    if MsgRead(id, data.as_mut_ptr().cast(), n, 16) < 0 {
        return kercall::tls::get_errno();
    }
    if d.controller {
        if !p.worker_open {
            return e::EIO;
        }
        for b in data {
            if let Some(sig) = p.input(b) {
                p.signal(sig);
            }
        }
        MsgReply(id, n as c_long, null(), 0);
    } else {
        if !p.controller_open {
            return e::EIO;
        }
        if p.output.len() >= OUTPUT_MAX {
            if nonblock != 0 {
                return e::EAGAIN;
            }
        } else if nonblock != 0 {
            let take = n.min(OUTPUT_MAX - p.output.len());
            for &b in &data[..take] {
                p.put(b);
            }
            MsgReply(id, take as c_long, null(), 0);
            p.settle();
            return RESMGR_NOREPLY;
        }
        p.worker_writes.push_back(Writer {
            rcvid: id,
            data,
            done: 0,
        });
    }
    p.settle();
    RESMGR_NOREPLY
}

unsafe extern "C" fn io_notify(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this manager.
    let d = unsafe { dev(ocb) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    let (input, side) = if d.controller {
        (!p.output.is_empty() || !p.worker_open, 1)
    } else {
        (p.input_count() > 0 || !p.controller_open, 0)
    };
    let output = d.controller || p.output.len() < OUTPUT_MAX;
    let trig = c_int::from(input) | c_int::from(output) << 1;
    // SAFETY: the framework's function, with this side's entries.
    unsafe {
        iofunc_notify(
            ctp,
            msg,
            p.notify[side].as_mut_ptr(),
            trig,
            null(),
            null_mut(),
        )
    }
}

/// The reply of a devctl with `data`.
unsafe fn devctl_reply(ctp: *mut Context, data: &[u8]) -> c_int {
    let head = DevctlReply {
        zero: 0,
        ret_val: 0,
        nbytes: data.len() as u32,
        zero2: 0,
    };
    let mut out = vec![0u8; size_of::<DevctlReply>() + data.len()];
    // SAFETY: the buffer holds the header.
    unsafe { out.as_mut_ptr().cast::<DevctlReply>().write_unaligned(head) };
    out[size_of::<DevctlReply>()..].copy_from_slice(data);
    // SAFETY: the context is the framework's.
    MsgReply(unsafe { rcvid(ctp) }, 0, out.as_ptr().cast(), out.len());
    RESMGR_NOREPLY
}

unsafe extern "C" fn io_devctl(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a devctl message.
    let m = unsafe { msg.cast::<Devctl>().read_unaligned() };
    // SAFETY: the payload follows the header, in the receive buffer.
    let data = unsafe { msg.add(size_of::<Devctl>()) };
    // SAFETY: an OCB of this manager.
    let d = unsafe { dev(ocb) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    // SAFETY: the payload's words, as each command has them.
    let word = |at: usize| unsafe { data.add(at).cast::<u32>().read_unaligned() };
    // SAFETY: the OCB, for `pty_devctl`'s access check.
    let writer = unsafe { (*ocb.cast::<IofuncOcb>()).ioflag } as u32 & IO_FLAG_MASK != 0;
    let answer = |v: i32| v.to_le_bytes();
    // SAFETY: `ctp` is the context of this message.
    unsafe {
        match m.dcmd as u32 & 0xffff {
            // TIOCGETA.
            0x7413 => {
                let mut t = [0u8; TERMIOS_SIZE];
                for (i, v) in [p.iflag, p.oflag, p.cflag, p.lflag].iter().enumerate() {
                    t[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
                t[16..16 + NCCS].copy_from_slice(&p.cc);
                t[36..40].copy_from_slice(&p.ispeed.to_le_bytes());
                t[40..44].copy_from_slice(&p.ospeed.to_le_bytes());
                devctl_reply(ctp, &t)
            }
            // TIOCSETA, TIOCSETAW, TIOCSETAF (which flushes input).
            c @ 0x7414..=0x7416 => {
                let canon = p.canonical();
                (p.iflag, p.oflag, p.cflag, p.lflag) = (word(0), word(4), word(8), word(12));
                core::ptr::copy_nonoverlapping(data.add(16), p.cc.as_mut_ptr(), NCCS);
                (p.ispeed, p.ospeed) = (word(36), word(40));
                if c == 0x7416 {
                    p.line.clear();
                    p.ready.clear();
                    p.lines.clear();
                }
                if canon && !p.canonical() {
                    // What was being edited is input now.
                    let line = core::mem::take(&mut p.line);
                    p.ready.extend(line);
                    p.lines.clear();
                }
                p.settle();
                devctl_reply(ctp, &[])
            }
            // TIOCGWINSZ, TIOCSWINSZ.
            0x7468 => devctl_reply(ctp, &p.winsize),
            0x7467 => {
                core::ptr::copy_nonoverlapping(data, p.winsize.as_mut_ptr(), 8);
                p.signal(SIGWINCH);
                devctl_reply(ctp, &[])
            }
            // TIOCGPGRP, TIOCSPGRP.
            0x7477 => devctl_reply(ctp, &answer(p.pgrp)),
            0x7476 => {
                p.pgrp = word(0) as i32;
                devctl_reply(ctp, &[])
            }
            // DCMD_CHR_TCGETSID, _TCSETSID: the caller leads its process
            // group too.
            0x0307 => devctl_reply(ctp, &answer(p.sid)),
            0x0308 => {
                p.sid = word(0) as i32;
                p.pgrp = p.sid;
                devctl_reply(ctp, &[])
            }
            // DCMD_CHR_ISATTY: not on a controller (QNX's `ptsname()`).
            0x0318 if d.controller => e::ENOTTY,
            // TIOCDRAIN, DCMD_CHR_ISATTY.
            0x745e | 0x0318 => devctl_reply(ctp, &[]),
            // TIOCOUTQ.
            0x7473 => devctl_reply(ctp, &answer(p.output.len() as i32)),
            // TIOCFLUSH: 0 input, 1 output, 2 both.
            0x7410 => {
                let which = word(0);
                if which != 1 {
                    p.line.clear();
                    p.ready.clear();
                    p.lines.clear();
                }
                if which != 0 {
                    p.output.clear();
                }
                p.settle();
                devctl_reply(ctp, &[])
            }
            // FIONREAD.
            0x667f => {
                let n = if d.controller {
                    p.output.len()
                } else {
                    p.input_count()
                };
                devctl_reply(ctp, &answer(n as i32))
            }
            // TIOCPTMASTER (libsys): whether this is a controller.
            0x741c => {
                if d.controller {
                    devctl_reply(ctp, &[])
                } else {
                    e::ENOTTY
                }
            }
            // FIODGNAME (libsys): the worker's name under /dev.
            0x6678 => {
                let mut name = [0u8; 32];
                let n = p.name();
                name[..n.len()].copy_from_slice(n.as_bytes());
                devctl_reply(ctp, &name)
            }
            // grantpt() (`pty_devctl`): the caller's, mode 0620.
            0x032a => {
                if !d.controller || !writer {
                    return e::EACCES;
                }
                let mut cred: resmgr::abi::ClientInfo = core::mem::zeroed();
                let r = iofunc_client_info(ctp, 0, &raw mut cred);
                if r != 0 {
                    return r;
                }
                let worker = &mut *DEVS[p.index * 2 + 1];
                worker.attr.uid = cred.ruid;
                worker.attr.gid = cred.rgid;
                worker.attr.mode = worker.attr.mode & !0o777 | 0o620;
                devctl_reply(ctp, &[])
            }
            // The same with explicit settings: uid, gid, mode.
            0x032b => {
                if !d.controller || !writer {
                    return e::EACCES;
                }
                let worker = &mut *DEVS[p.index * 2 + 1];
                worker.attr.uid = word(0);
                worker.attr.gid = word(4);
                worker.attr.mode = worker.attr.mode & !0o777 | word(8) & 0o777;
                devctl_reply(ctp, &[])
            }
            _ => {
                let r = iofunc_devctl_default(ctp, msg, ocb.cast());
                if r == RESMGR_DEFAULT { e::ENOTTY } else { r }
            }
        }
    }
}

/// A client asked to leave a held read or write.
unsafe extern "C" fn io_unblock(ctp: *mut Context, _msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this manager, and the context's message.
    let (d, id) = unsafe { (dev(ocb), rcvid(ctp)) };
    // SAFETY: as above.
    let p = unsafe { &mut *d.pty };
    let held = p.controller_reads.iter().any(|r| r.rcvid == id)
        || p.worker_reads.iter().any(|r| r.rcvid == id)
        || p.worker_writes.iter().any(|w| w.rcvid == id);
    p.controller_reads.retain(|r| r.rcvid != id);
    p.worker_reads.retain(|r| r.rcvid != id);
    p.worker_writes.retain(|w| w.rcvid != id);
    if held {
        MsgError(id, e::EINTR);
    }
    RESMGR_NOREPLY
}

static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };
/// The devices: each pty's controller, then its worker.
static mut DEVS: Vec<*mut Dev> = Vec::new();

fn fail(what: &str) -> ! {
    eprintln!("pty: {what}: {}", std::io::Error::last_os_error());
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let count = args
        .iter()
        .position(|a| a == "-n")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(8usize)
        .clamp(1, 256);
    // SAFETY: the framework's functions, with this program's statics,
    // which only this one thread touches.
    unsafe {
        let dpp = dispatch_create();
        if dpp.is_null() {
            fail("dispatch_create");
        }
        iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
        (*(&raw mut CONNECT)).funcs[0] = Some(io_open);
        let io = &mut *(&raw mut IO);
        io.funcs[IO_READ] = Some(io_read);
        io.funcs[IO_WRITE] = Some(io_write);
        io.funcs[IO_CLOSE_OCB] = Some(io_close_ocb);
        io.funcs[IO_NOTIFY] = Some(io_notify);
        io.funcs[IO_DEVCTL] = Some(io_devctl);
        io.funcs[IO_UNBLOCK] = Some(io_unblock);
        let attr = ResmgrAttr {
            flags: 0,
            nparts_max: 1,
            msg_max_size: 4096,
            other_func: None,
            reserved: [0; 4],
        };
        for i in 0..count {
            let pty = Box::into_raw(Box::new(Pty::new(i)));
            for (prefix, controller) in [("pty", true), ("tty", false)] {
                let d: *mut Dev = Box::into_raw(Box::new(core::mem::zeroed()));
                (*d).pty = pty;
                (*d).controller = controller;
                iofunc_attr_init(&raw mut (*d).attr, S_IFCHR | 0o666, null(), null());
                (*(&raw mut DEVS)).push(d);
                let path = format!("/dev/{}\0", name(prefix, i));
                let id = resmgr_attach(
                    dpp,
                    &attr,
                    path.as_ptr().cast::<c_char>(),
                    0,
                    0,
                    &raw const CONNECT,
                    &raw const IO,
                    d.cast::<c_void>(),
                );
                if id < 0 {
                    fail("resmgr_attach");
                }
            }
        }
        let ctp = dispatch_context_alloc(dpp);
        if ctp.is_null() {
            fail("dispatch_context_alloc");
        }
        loop {
            if dispatch_block(ctp).is_null() {
                fail("dispatch_block");
            }
            dispatch_handler(ctp);
        }
    }
}

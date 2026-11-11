//! Kernel pipes (QNX 8 `PipeOpen()`, `PipeCtl()`, `PipeClose()`;
//! `<sys/kpipe.h>`), as procnto's `ker_pipe_*` and `nto_pipe_msg_send`.
//!
//! A pipe manager (QNX's `kpipe`) answers a client's open of a pipe by
//! binding the client's connection to a pipe in the kernel:
//! - `PipeOpen(scoid, -1, cfg, flags)` makes a pipe (`nto_pipe_cfg_t`:
//!   buffer size, atomic write size, write notification threshold, simplex
//!   or duplex, source buffer) and binds the connection of client `scoid`
//!   (one to the manager's channel) to it; the result is the pipe's ID.
//!   `PipeOpen(scoid, id, cfg, flags)` binds another connection to pipe
//!   `id` (a dup, `pipe()`'s other end). `flags`: `_IO_FLAG_RD`/`_WR`,
//!   `O_NONBLOCK`. A simplex pipe has one buffer: an end reads from its
//!   source buffer and writes to the other, so the manager gives the two
//!   ends of `pipe()` opposite source buffers.
//! - `PipeClose(id)`: the manager forgets the pipe; it goes with its last
//!   connection.
//! - `PipeCtl(scoid, ctl)`: `_NTO_PIPE_CTL_NOTIFY` (an `ionotify()` on the
//!   client's behalf), `_GETTIME`, `_SETFLAGS`/`_GETFLAGS` (`O_NONBLOCK`),
//!   `_UNBLOCK` (the client's blocked threads), `_DISABLE` (the connection
//!   no longer reads or writes, as when it closes).
//! - A bound connection's `_IO_READ` and `_IO_WRITE` never reach the
//!   manager: the kernel copies the data (`msg_send`). A read of an empty
//!   pipe returns 0 without writers, EAGAIN if non-blocking, else waits
//!   (`STATE_PIPE`); a write without readers is SIGPIPE and EPIPE; up to
//!   the atomic size a write goes in whole or waits, beyond it as much as
//!   fits, and non-blocking writes take what fits at once (EAGAIN if
//!   nothing). `TimerTimeout()` and signals end the waits. Other messages
//!   (stat, lseek, dup, close) still go to the manager.
//! - Closing a bound connection ends its reading or writing: the last
//!   writer's close wakes the readers (end of file), the last reader's the
//!   writers (EPIPE), and armed notifications fire.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EBADF, EFAULT, EINVAL, EIO, EPERM, ESPIPE, ESRCH};
use super::event::{self, SigEvent};
use super::process;
use super::umem::{self, Space};
use crate::sched::{self, State, ThreadRef};
use crate::sync::IrqSpinLock;

use crate::ker::errno::EPIPE;
const SIGPIPE: usize = 13;
const IO_READ: u16 = 0x101;
const IO_WRITE: u16 = 0x102;
const XTYPE_NONE: u8 = 0;
const XTYPE_OFFSET: u8 = 5;

/// Binding flags: the access, `O_NONBLOCK`, the source buffer.
pub const RD: u16 = 0x1;
pub const WR: u16 = 0x2;
const NONBLOCK: u16 = 0x80;
const SRC1: u16 = 0x4000;
/// `_NTO_COF_PIPE`: a connection bound to a pipe (`ConnectServerInfo()`).
pub const COF_PIPE: u32 = 0x400;

const CFG_DUPLEX: u32 = 0x4;
const CFG_SIZE: usize = 64;

const CTL_NOTIFY: u32 = 1;
const CTL_GETTIME: u32 = 2;
const CTL_SETFLAGS: u32 = 3;
const CTL_GETFLAGS: u32 = 4;
const CTL_UNBLOCK: u32 = 5;
const CTL_DISABLE: u32 = 6;
/// OsCraft's, for the native manager: { id, len, bytes to read, atomic
/// size } of the connection, for `stat()`, `FIONREAD` and `PIPE_BUF`.
const CTL_INFO: u32 = 0x100;

/// `_NOTIFY_COND_INPUT`, `_OUTPUT`, `_EXTEN` (`<sys/iomsg.h>`).
const COND_INPUT: u32 = 0x1000_0000;
const COND_OUTPUT: u32 = 0x2000_0000;
const COND_EXTEN: u32 = 0x8000_0000;
/// `_NOTIFY_CONDE_HUP`.
const CONDE_HUP: u32 = 0x40;
/// `SI_NOTIFY`: libc's `poll()` events.
const SI_NOTIFY: i16 = -6;

struct Buffer {
    data: VecDeque<u8>,
    size: usize,
    /// No such buffer (the second of a simplex pipe), or disabled.
    disabled: bool,
    readers: u16,
    writers: u16,
    /// Threads waiting to read from it, to write to it.
    waiting_readers: VecDeque<ThreadRef>,
    waiting_writers: VecDeque<ThreadRef>,
    /// Armed notifications: [input, output].
    notify: [Vec<Notification>; 2],
}

impl Buffer {
    fn free(&self) -> usize {
        // Over its size for a moment if a failed read put its data back
        // (`read`) after writers had filled the room.
        self.size.saturating_sub(self.data.len())
    }
}

#[derive(Clone)]
struct Notification {
    /// The client connection it is for (its address: connections of a
    /// QNX client may share a scoid), and the thread that armed it.
    key: usize,
    pid: i32,
    tid: u32,
    /// The data (input) or room (output) it waits for.
    count: u32,
    /// Its `ionotify()` conditions (`_NOTIFY_COND_EXTEN` gives the
    /// detailed ones too).
    flags: u32,
    event: SigEvent,
}

struct Inner {
    bufs: [Buffer; 2],
    /// Last read, last write or change (ns of the realtime clock).
    atime: u64,
    cmtime: u64,
}

pub struct Pipe {
    /// The manager that made it.
    server: i32,
    atomic: usize,
    wnotify: usize,
    inner: IrqSpinLock<Inner>,
}

/// A connection's binding (`Connection::pipe`).
pub struct Binding {
    pipe: Arc<Pipe>,
    flags: AtomicU16,
}

impl Binding {
    fn flags(&self) -> u16 {
        self.flags.load(Ordering::Relaxed)
    }

    /// The buffer this end reads from, and the one it writes to.
    fn read_buf(&self) -> usize {
        (self.flags() & SRC1 != 0) as usize
    }

    fn write_buf(&self) -> usize {
        1 - self.read_buf()
    }
}

/// Each manager's pipes by ID.
static PIPES: IrqSpinLock<BTreeMap<(i32, i32), Arc<Pipe>>> = IrqSpinLock::new(BTreeMap::new());
static NEXT_ID: IrqSpinLock<i32> = IrqSpinLock::new(1);
/// Threads blocked in a pipe, by tid.
static BLOCKED: IrqSpinLock<BTreeMap<u32, Arc<Pipe>>> = IrqSpinLock::new(BTreeMap::new());

fn now() -> u64 {
    crate::clock::tod_ns()
}

/// The connections of client `scoid` to the calling server's channel:
/// one, or several of a QNX client, whose `ConnectAttach()` without
/// `_NTO_COF_NOSHARE` gives a process's connections to one channel the
/// same server connection ID (they share the open file: `dup()`).
fn client_connections(scoid: i32) -> Result<Vec<Arc<super::channel::Connection>>, u32> {
    let me = process::current().pid;
    let p = process::by_scoid(me, scoid).ok_or(EBADF)?;
    let conns = p.connections_by_scoid(me, scoid);
    if conns.is_empty() {
        return Err(EBADF);
    }
    Ok(conns)
}

/// The connections of `scoid` bound to a pipe.
fn bound_connections(scoid: i32) -> Result<Vec<Arc<super::channel::Connection>>, u32> {
    let conns: Vec<_> = client_connections(scoid)?
        .into_iter()
        .filter(|c| c.pipe.lock().is_some())
        .collect();
    if conns.is_empty() {
        return Err(EBADF);
    }
    Ok(conns)
}

/// Connection `to` is a copy of `from` (a fork's or a spawn's inherited
/// descriptor, `dup2()`): bound to the same pipe as the same end, as the
/// copy of an open file. The manager's `PipeOpen()` for the copy (after
/// its `_IO_DUP`) then finds it bound.
pub fn copy_binding(from: &super::channel::Connection, to: &super::channel::Connection) {
    let Some(b) = from.pipe.lock().clone() else {
        return;
    };
    let flags = b.flags();
    let copy = Binding {
        pipe: b.pipe.clone(),
        flags: AtomicU16::new(flags),
    };
    {
        let mut inner = b.pipe.inner.lock();
        if flags & RD != 0 {
            inner.bufs[copy.read_buf()].readers += 1;
        }
        if flags & WR != 0 {
            inner.bufs[copy.write_buf()].writers += 1;
        }
    }
    *to.pipe.lock() = Some(Arc::new(copy));
}

/// `PipeOpen(scoid, pipe_id, cfg, flags)`: binds the connections of
/// `scoid` not bound yet: one, or the ones a QNX client shares the scoid
/// among (`dup()`'s `ConnectAttach()` without `_NTO_COF_NOSHARE`), of
/// which a `_IO_DUP` names one at a time. When all are bound already
/// (copies, `copy_binding`), one of them to pipe `pipe_id`, there is
/// nothing to do (a shared scoid may span connections to several pipes:
/// a shell's two pipelines); EBADF otherwise.
pub fn pipe_open(scoid: i32, pipe_id: i32, cfg: u64, flags: u32) -> KerResult {
    let mut c = [0u8; CFG_SIZE];
    umem::read(Space::current(), cfg, &mut c).map_err(|_| EFAULT)?;
    let u32_at = |o: usize| u32::from_le_bytes(c[o..o + 4].try_into().unwrap());
    let (bufsz, chunksz, wnotify, ty, source) = (u32_at(0), u32_at(4), u32_at(8), u32_at(12), u32_at(16));
    if pipe_id >= 0x10000 {
        return Err(EINVAL);
    }
    let me = process::current().pid;
    let conns = client_connections(scoid)?;
    let unbound: Vec<_> = conns.iter().filter(|c| c.pipe.lock().is_none()).cloned().collect();
    if unbound.is_empty() {
        if pipe_id < 0 {
            return Err(EBADF);
        }
        let pipe = PIPES.lock().get(&(me, pipe_id)).cloned().ok_or(ESRCH)?;
        let same = conns
            .iter()
            .any(|c| c.pipe.lock().as_ref().is_some_and(|b| Arc::ptr_eq(&b.pipe, &pipe)));
        return if same { Ok(pipe_id as u64) } else { Err(EBADF) };
    }
    let mut bflags = (flags & 3) as u16;
    if flags & nonblock_flag() != 0 {
        bflags |= NONBLOCK;
    }
    let (pipe, result) = if pipe_id < 0 {
        let duplex = ty & CFG_DUPLEX != 0;
        if !duplex && bflags & (RD | WR) == RD | WR {
            return Err(EPERM);
        }
        if bufsz == 0 || chunksz == 0 {
            return Err(EINVAL);
        }
        let wnotify = if wnotify == 0 || wnotify > chunksz { chunksz } else { wnotify };
        let buffer = |size: u32, disabled: bool| Buffer {
            data: VecDeque::new(),
            size: size as usize,
            disabled,
            readers: 0,
            writers: 0,
            waiting_readers: VecDeque::new(),
            waiting_writers: VecDeque::new(),
            notify: [Vec::new(), Vec::new()],
        };
        let id = {
            let mut next = NEXT_ID.lock();
            let id = *next;
            *next = if id >= 0xfffe { 1 } else { id + 1 };
            id
        };
        let t = now();
        let pipe = Arc::new(Pipe {
            server: me,
            atomic: chunksz as usize,
            wnotify: wnotify as usize,
            inner: IrqSpinLock::new(Inner {
                bufs: [buffer(bufsz, false), buffer(if duplex { bufsz } else { 0 }, !duplex)],
                atime: t,
                cmtime: t,
            }),
        });
        PIPES.lock().insert((me, id), pipe.clone());
        (pipe, id)
    } else {
        let pipe = PIPES.lock().get(&(me, pipe_id)).cloned().ok_or(ESRCH)?;
        if source >= 2 {
            return Err(EINVAL);
        }
        let inner = pipe.inner.lock();
        let src = source as usize;
        if (bflags & RD != 0 && inner.bufs[src].disabled) || (bflags & WR != 0 && inner.bufs[1 - src].disabled) {
            return Err(EPERM);
        }
        drop(inner);
        (pipe, pipe_id)
    };
    if source != 0 {
        bflags |= SRC1;
    }
    for conn in &unbound {
        let mut slot = conn.pipe.lock();
        if slot.is_some() {
            continue;
        }
        let b = Binding {
            pipe: pipe.clone(),
            flags: AtomicU16::new(bflags),
        };
        {
            let mut inner = pipe.inner.lock();
            if bflags & RD != 0 {
                inner.bufs[b.read_buf()].readers += 1;
            }
            if bflags & WR != 0 {
                inner.bufs[b.write_buf()].writers += 1;
            }
        }
        *slot = Some(Arc::new(b));
    }
    Ok(result as u64)
}

/// `O_NONBLOCK` as the calling manager numbers it: QNX's `kpipe`, or a
/// native one.
fn nonblock_flag() -> u32 {
    match process::current().personality() {
        process::Personality::Qnx => 0x80,
        process::Personality::Native => 0x4,
    }
}

/// `PipeClose(pipe_id)`.
pub fn pipe_close(pipe_id: i32) -> KerResult {
    let me = process::current().pid;
    PIPES.lock().remove(&(me, pipe_id)).map(|_| 0).ok_or(ESRCH)
}

// ANCHOR: pipe_msg_send
/// Whether a message on connection `conn` is a pipe's, and if so its
/// result (`MsgSendv()` on a bound connection).
///
/// The binding is taken (an `Arc`) under the connection's lock, which is
/// then dropped: the header and the data are copied with no lock held, as
/// a copy may fault pages in (review IPC-21).
pub fn msg_send(conn: &super::channel::Connection, smsg: &umem::Buf, rmsg: &umem::Buf) -> Option<KerResult> {
    let b = conn.pipe.lock().clone()?;
    let mut h = [0u8; 16];
    let n = umem::copy(&umem::Buf::kernel_mut(&mut h), smsg).unwrap_or(0) as usize;
    let ty = u16::from_le_bytes([h[0], h[1]]);
    if n < 2 || !matches!(ty, IO_READ | IO_WRITE) {
        return None;
    }
    if n < 16 {
        return Some(Err(super::errno::EBADMSG));
    }
    let xtype = h[8];
    if !matches!(xtype, XTYPE_NONE | XTYPE_OFFSET) {
        return None;
    }
    if xtype == XTYPE_OFFSET {
        return Some(Err(ESPIPE));
    }
    let nbytes = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
    let (rbuf, wbuf) = (b.read_buf(), b.write_buf());
    Some(if ty == IO_READ {
        read(&b.pipe, conn, &b.flags, rbuf, nbytes, rmsg)
    } else {
        write(&b.pipe, conn, &b.flags, wbuf, nbytes, &smsg.skip(16))
    })
}
// ANCHOR_END: pipe_msg_send

/// Block the calling thread in `pipe` (its lock passed in, the thread
/// already in a waiter list); returns the wait's error.
fn wait(pipe: &Arc<Pipe>, inner: crate::sync::IrqSpinGuard<'_, Inner>) -> Result<(), u32> {
    let me = sched::current();
    BLOCKED.lock().insert(me.tid, pipe.clone());
    sched::prepare_block(State::Pipe);
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(State::Pipe);
    drop(inner);
    sched::block();
    crate::timeout::disarm(t);
    BLOCKED.lock().remove(&me.tid);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Ok(()),
        e => Err(e),
    }
}

fn read(pipe: &Arc<Pipe>, conn: &super::channel::Connection, flags: &AtomicU16, idx: usize, nbytes: usize, rmsg: &umem::Buf) -> KerResult {
    if flags.load(Ordering::Relaxed) & RD == 0 {
        return Err(EBADF);
    }
    let me = sched::current();
    let (data, fired) = loop {
        let mut inner = pipe.inner.lock();
        let b = &mut inner.bufs[idx];
        if b.disabled {
            return Err(EBADF);
        }
        if nbytes == 0 {
            return Ok(0);
        }
        if b.data.is_empty() {
            if b.writers == 0 {
                return Ok(0);
            }
            if flags.load(Ordering::Relaxed) & NONBLOCK != 0 {
                return Err(EAGAIN);
            }
            b.waiting_readers.push_back(me.arc());
            me.pipe_conn.store(conn as *const _ as usize, Ordering::Relaxed);
            wait(pipe, inner)?;
            continue;
        }
        let n = nbytes.min(b.data.len()).min(rmsg.total() as usize);
        let data: Vec<u8> = b.data.drain(..n).collect();
        let free = b.free();
        let fired = if let Some(w) = b.waiting_writers.pop_front() {
            sched::wake(&w);
            Vec::new()
        } else if free >= pipe.wnotify {
            take_fired(b, 1, free as u32, false)
        } else {
            Vec::new()
        };
        inner.atime = now();
        break (data, fired);
    };
    deliver(pipe, fired);
    // A reader's buffer that faults gets EFAULT, and the data stays in the
    // pipe, in front of what was written meanwhile (procnto, measured,
    // review IPC-21: even when part of it was copied).
    if umem::copy_preemptible(rmsg, &umem::Buf::kernel(&data)).is_err() {
        let mut inner = pipe.inner.lock();
        let b = &mut inner.bufs[idx];
        for &byte in data.iter().rev() {
            b.data.push_front(byte);
        }
        return Err(EFAULT);
    }
    Ok(data.len() as u64)
}

// ANCHOR: pipe_write
fn write(pipe: &Arc<Pipe>, conn: &super::channel::Connection, flags: &AtomicU16, idx: usize, nbytes: usize, src: &umem::Buf) -> KerResult {
    if flags.load(Ordering::Relaxed) & WR == 0 {
        return Err(EBADF);
    }
    if nbytes == 0 {
        return Ok(0);
    }
    if (src.total() as usize) < nbytes {
        return Err(super::errno::EBADMSG);
    }
    let me = sched::current();
    let mut done = 0usize;
    loop {
        let nonblock = flags.load(Ordering::Relaxed) & NONBLOCK != 0;
        let mut inner = pipe.inner.lock();
        let b = &mut inner.bufs[idx];
        if b.readers == 0 {
            drop(inner);
            sigpipe();
            return Err(EPIPE);
        }
        if b.disabled {
            return Err(EIO);
        }
        let left = nbytes - done;
        // Up to the atomic size the write goes in whole; beyond it, as
        // much as fits (at least a byte), an atomic size at most.
        let want = if nbytes <= pipe.atomic { left } else { b.free().max(1).min(pipe.atomic).min(left) };
        if b.free() < want {
            if nonblock {
                return if done > 0 { Ok(done as u64) } else { Err(EAGAIN) };
            }
            b.waiting_writers.push_back(me.arc());
            me.pipe_conn.store(conn as *const _ as usize, Ordering::Relaxed);
            match wait(pipe, inner) {
                Ok(()) => continue,
                Err(e) if done > 0 && e != EPIPE => return Ok(done as u64),
                Err(e) => return Err(e),
            }
        }
        let n = b.free().min(left);
        drop(inner);
        let mut chunk = alloc::vec![0u8; n];
        let got = umem::copy_preemptible(&umem::Buf::kernel_mut(&mut chunk), &src.skip(done as u64)).map_err(|_| EFAULT)? as usize;
        if got < n {
            return Err(EFAULT);
        }
        let mut inner = pipe.inner.lock();
        let b = &mut inner.bufs[idx];
        if b.readers == 0 {
            drop(inner);
            sigpipe();
            return if done > 0 { Ok(done as u64) } else { Err(EPIPE) };
        }
        let n = n.min(b.free());
        b.data.extend(&chunk[..n]);
        done += n;
        let avail = b.data.len() as u32;
        let fired = if let Some(r) = b.waiting_readers.pop_front() {
            sched::wake(&r);
            Vec::new()
        } else {
            take_fired(b, 0, avail, false)
        };
        inner.cmtime = now();
        drop(inner);
        deliver(pipe, fired);
        if done == nbytes || nonblock {
            return Ok(done as u64);
        }
    }
}
// ANCHOR_END: pipe_write

/// SIGPIPE to the calling process.
fn sigpipe() {
    let p = process::current();
    let info = super::signal::SigInfo {
        code: 0, // SI_USER
        pid: p.pid,
        ..Default::default()
    };
    super::signal::post(p, SIGPIPE, info);
}

/// Take the armed notifications of `b`'s list `which` (0 input, 1
/// output) that `count` bytes (of data, or of room) satisfy, or all of
/// them for a hang-up (`hup`), with their events made ready.
fn take_fired(b: &mut Buffer, which: usize, count: u32, hup: bool) -> Vec<Notification> {
    let mut fired = Vec::new();
    b.notify[which].retain(|n| {
        if hup || count >= n.count {
            let mut n = n.clone();
            if n.event.code == SI_NOTIFY {
                let cond = COND_INPUT << which;
                let v = n.event.value as u32 | cond;
                n.event.value = (n.event.value & !0xffff_ffff) | if n.flags & COND_EXTEN != 0 { (v | COND_EXTEN) as u64 } else { v as u64 };
            } else if n.event.notify & event::FLAG_UPDATEABLE != 0 {
                n.event.value = (n.event.value & !0xffff_ffff) | count as u64;
            }
            fired.push(n);
            false
        } else {
            true
        }
    });
    fired
}

/// Deliver fired notifications (to their processes, as from the pipe's
/// manager, which their registered events allow).
fn deliver(pipe: &Pipe, fired: Vec<Notification>) {
    for n in fired {
        let Some(p) = process::lookup(n.pid) else {
            continue;
        };
        if let Ok(ev) = event::resolve(&p, pipe.server, &n.event) {
            let t = sched::thread_of(&p, n.tid);
            let _ = event::deliver(&p, t, &ev, 0);
        }
    }
}

/// The threads blocked through connection `conn`, of the given waiter
/// lists, unblocked with `err`.
fn unblock_conn(b: &mut Buffer, conn: usize, readers: bool, err: u32) {
    let list = if readers { &mut b.waiting_readers } else { &mut b.waiting_writers };
    list.retain(|t| {
        if t.pipe_conn.load(Ordering::Relaxed) == conn {
            t.wake_error.store(err, Ordering::Relaxed);
            sched::wake(t);
            false
        } else {
            true
        }
    });
}

/// The connection no longer reads or writes (closed, or `_DISABLE`):
/// its threads wait no more, its notifications go, and the last writer
/// or reader wakes the other side.
fn disable(conn: &super::channel::Connection, b: &Binding) {
    let key = conn as *const _ as usize;
    let flags = b.flags.fetch_and(!(RD | WR), Ordering::Relaxed);
    let pipe = &b.pipe;
    let mut fired = Vec::new();
    {
        let mut inner = pipe.inner.lock();
        if flags & WR != 0 {
            let buf = &mut inner.bufs[b.write_buf()];
            unblock_conn(buf, key, false, EBADF);
            for list in buf.notify.iter_mut() {
                list.retain(|n| n.key != key);
            }
            buf.writers -= 1;
            if buf.writers == 0 {
                for t in buf.waiting_readers.drain(..) {
                    sched::wake(&t);
                }
                fired.extend(take_fired(buf, 0, 0, true));
            }
        }
        if flags & RD != 0 {
            let buf = &mut inner.bufs[b.read_buf()];
            unblock_conn(buf, key, true, EBADF);
            for list in buf.notify.iter_mut() {
                list.retain(|n| n.key != key);
            }
            buf.readers -= 1;
            if buf.readers == 0 {
                for t in buf.waiting_writers.drain(..) {
                    sched::wake(&t);
                }
                fired.extend(take_fired(buf, 1, 0, true));
            }
        }
    }
    deliver(pipe, fired);
}

/// Connection `conn` is going: unbind it (`Connection`'s drop).
pub fn disconnect(conn: &super::channel::Connection) {
    let b = conn.pipe.lock().take();
    if let Some(b) = b {
        disable(conn, &b);
    }
}

/// The connection a `PipeCtl()` is about. A client's `dup()`s of different
/// pipes share one scoid, so the scoid alone doesn't say: it is the one the
/// client is waiting for this reply on (the one named by `tid` in a
/// `_NTO_PIPE_CTL_NOTIFY`), and without a client waiting, the first.
fn served_connection(
    scoid: i32,
    conns: &[Arc<super::channel::Connection>],
    space: Space,
    ctl: u64,
    id: u32,
) -> Arc<super::channel::Connection> {
    let mut tid = [0u8; 4];
    let tid = (id == CTL_NOTIFY && umem::read(space, ctl + 0x18, &mut tid).is_ok()).then(|| u32::from_le_bytes(tid));
    process::by_scoid(process::current().pid, scoid)
        .into_iter()
        .flat_map(|p| crate::sched::threads_of(&p))
        .filter(|t| tid.is_none_or(|tid| t.ptid == tid))
        .filter_map(|t| super::msg::reply_connection(&t))
        .find(|c| conns.iter().any(|b| Arc::ptr_eq(b, c)))
        .unwrap_or_else(|| conns[0].clone())
}

/// `PipeCtl(scoid, ctl)`.
pub fn pipe_ctl(scoid: i32, ctl: u64) -> KerResult {
    let space = Space::current();
    let mut h = [0u8; 8];
    umem::read(space, ctl, &mut h).map_err(|_| EFAULT)?;
    let (id, len) = (u32::from_le_bytes(h[0..4].try_into().unwrap()), u32::from_le_bytes(h[4..8].try_into().unwrap()));
    if len < 8 {
        return Err(EINVAL);
    }
    let conns = bound_connections(scoid)?;
    let conn = served_connection(scoid, &conns, space, ctl, id);
    let need = |n: u32| if len < n { Err(EINVAL) } else { Ok(()) };
    match id {
        CTL_GETTIME => {
            need(0x20)?;
            let (atime, cmtime) = {
                let slot = conn.pipe.lock();
                let inner = slot.as_ref().ok_or(EBADF)?.pipe.inner.lock();
                (inner.atime, inner.cmtime)
            };
            let mut out = [0u8; 24];
            out[0..8].copy_from_slice(&atime.to_le_bytes());
            out[8..16].copy_from_slice(&cmtime.to_le_bytes());
            out[16..24].copy_from_slice(&cmtime.to_le_bytes());
            umem::write(space, ctl + 8, &out).map_err(|_| EFAULT)?;
            Ok(0)
        }
        CTL_SETFLAGS | CTL_GETFLAGS => {
            need(0x10)?;
            let mut f = [0u8; 4];
            umem::read(space, ctl + 8, &mut f).map_err(|_| EFAULT)?;
            let asked = u32::from_le_bytes(f);
            let slot = conn.pipe.lock();
            let b = slot.as_ref().ok_or(EBADF)?;
            let flags = b.flags();
            // The access as O_ACCMODE, O_NONBLOCK.
            let mut cur = match (flags & RD != 0, flags & WR != 0) {
                (true, true) => 2,
                (false, true) => 1,
                _ => 0,
            };
            if flags & NONBLOCK != 0 {
                cur |= nonblock_flag();
            }
            drop(slot);
            if id == CTL_SETFLAGS {
                let (pipe, end) = {
                    let slot = conn.pipe.lock();
                    let b = slot.as_ref().ok_or(EBADF)?;
                    (b.pipe.clone(), b.flags() & (RD | WR))
                };
                for c in &conns {
                    let slot = c.pipe.lock();
                    if let Some(b) = slot.as_ref() {
                        let flags = b.flags();
                        // The same open file: another pipe's isn't.
                        if !Arc::ptr_eq(&b.pipe, &pipe) || flags & (RD | WR) != end {
                            continue;
                        }
                        let new = if asked & nonblock_flag() != 0 { flags | NONBLOCK } else { flags & !NONBLOCK };
                        b.flags.store(new, Ordering::Relaxed);
                    }
                }
            }
            umem::write(space, ctl + 12, &(cur as u32).to_le_bytes()).map_err(|_| EFAULT)?;
            Ok(0)
        }
        CTL_UNBLOCK => {
            for conn in &conns {
                let slot = conn.pipe.lock();
                let Some(b) = slot.as_ref() else { continue };
                let key = &**conn as *const _ as usize;
                let mut inner = b.pipe.inner.lock();
                let (r, w) = (b.read_buf(), b.write_buf());
                unblock_conn(&mut inner.bufs[w], key, false, super::errno::EINTR);
                unblock_conn(&mut inner.bufs[r], key, true, super::errno::EINTR);
            }
            Ok(0)
        }
        CTL_DISABLE => {
            for conn in &conns {
                let slot = conn.pipe.lock();
                let Some(b) = slot.as_ref() else { continue };
                disable(conn, b);
            }
            Ok(0)
        }
        CTL_INFO => {
            need(0x10)?;
            let slot = conn.pipe.lock();
            let b = slot.as_ref().ok_or(EBADF)?;
            let bytes = b.pipe.inner.lock().bufs[b.read_buf()].data.len() as u32;
            let atomic = b.pipe.atomic as u32;
            drop(slot);
            umem::write(space, ctl + 8, &bytes.to_le_bytes()).map_err(|_| EFAULT)?;
            umem::write(space, ctl + 12, &atomic.to_le_bytes()).map_err(|_| EFAULT)?;
            Ok(0)
        }
        CTL_NOTIFY => {
            need(0x50)?;
            let mut m = [0u8; 0x50];
            umem::read(space, ctl, &mut m).map_err(|_| EFAULT)?;
            notify(&conn, &m)
        }
        _ => Err(super::errno::ENOSYS),
    }
}

/// `_NTO_PIPE_CTL_NOTIFY` { id, len, action, pid64, tid, condflags,
/// extraflags, event, replybuf }: the pipe's side of a client's
/// `ionotify()`. The action is libc's mapped: 6 poll, 3 poll and arm if
/// nothing is ready (`_NOTIFY_ACTION_POLLARM`, `_CONDARM`), 1 arm
/// (`_TRANSPARENT`). The ready conditions go to `*replybuf`.
fn notify(conn: &super::channel::Connection, m: &[u8]) -> KerResult {
    let u32_at = |o: usize| u32::from_le_bytes(m[o..o + 4].try_into().unwrap());
    let (action, tid, condflags) = (u32_at(8), u32_at(0x18), u32_at(0x1c));
    let pid64 = u64::from_le_bytes(m[0x10..0x18].try_into().unwrap());
    let pid = if pid64 == 0 { process::current().pid } else { pid64 as u32 as i32 };
    let event = SigEvent::from_bytes(m[0x28..0x48].try_into().unwrap());
    let replybuf = u64::from_le_bytes(m[0x48..0x50].try_into().unwrap());
    // The conditions asked for (the detailed ones too), those the
    // connection's access allows.
    let mut conds = (condflags | (condflags & 7) << 28) & (COND_INPUT | COND_OUTPUT | COND_EXTEN);
    let slot = conn.pipe.lock();
    let b = slot.as_ref().ok_or(EBADF)?;
    let flags = b.flags();
    if flags & RD == 0 {
        conds &= !COND_INPUT;
    }
    if flags & WR == 0 {
        conds &= !COND_OUTPUT;
    }
    let pipe = b.pipe.clone();
    let (r, w) = (b.read_buf(), b.write_buf());
    drop(slot);
    let key = conn as *const _ as usize;
    let mut ready = 0;
    let hup;
    {
        let mut inner = pipe.inner.lock();
        // Only for a FreeBSD client: procnto never says it (measured).
        let native = process::lookup(pid).is_some_and(|p| p.personality() == process::Personality::Native);
        hup = native
            && ((flags & RD != 0 && inner.bufs[r].writers == 0) || (flags & WR != 0 && inner.bufs[w].readers == 0));
        if conds & COND_INPUT != 0 {
            let buf = &inner.bufs[r];
            if !buf.data.is_empty() || buf.writers == 0 {
                ready |= COND_INPUT;
            }
        }
        if conds & COND_OUTPUT != 0 {
            let buf = &inner.bufs[w];
            if buf.free() >= pipe.wnotify || buf.readers == 0 {
                ready |= COND_OUTPUT;
            }
        }
        let arm = match action & 7 {
            1 => true,
            3 => ready == 0 && !hup,
            _ => false,
        };
        if arm && event.notify & 0xff != 0 {
            for (which, cond, idx, count) in [(0usize, COND_INPUT, r, 1u32), (1, COND_OUTPUT, w, pipe.wnotify as u32)] {
                if conds & cond == 0 {
                    continue;
                }
                let list = &mut inner.bufs[idx].notify[which];
                list.retain(|n| !(n.key == key && n.tid == tid));
                list.push(Notification {
                    key,
                    pid,
                    tid,
                    count,
                    flags: condflags,
                    event,
                });
            }
        }
    }
    let mut result = ready;
    if condflags & COND_EXTEN != 0 {
        // A hangup needn't be asked for, as `pipe_poll()` on FreeBSD: the
        // other end is gone.
        if hup {
            result |= CONDE_HUP;
        }
        if result != 0 {
            result |= result >> 28 | COND_EXTEN;
        }
    }
    umem::write(Space::current(), replybuf, &result.to_le_bytes()).map_err(|_| EFAULT)?;
    Ok(4)
}

/// Make thread `t`, blocked in a pipe, leave with `err`.
pub fn unblock(t: &sched::Thread, err: u32) -> bool {
    let Some(pipe) = BLOCKED.lock().get(&t.tid).cloned() else {
        return false;
    };
    let mut found = false;
    {
        let mut inner = pipe.inner.lock();
        for b in inner.bufs.iter_mut() {
            for list in [&mut b.waiting_readers, &mut b.waiting_writers] {
                if let Some(i) = list.iter().position(|w| core::ptr::eq(&**w, t)) {
                    list.remove(i);
                    found = true;
                }
            }
        }
    }
    if found {
        t.wake_error.store(err, Ordering::Relaxed);
        sched::wake(t);
    }
    found
}

/// Process `pid` has ended: the pipes it made as a manager are forgotten
/// (they go with their last connections).
pub fn process_exited(pid: i32) {
    PIPES.lock().retain(|&(server, _), _| server != pid);
}

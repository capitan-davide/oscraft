//! POSIX message queues in the kernel (`MsgQueueOpen()` & co., QNX 8).
//!
//! Contract, from the QNX 8.0 library reference (`MsgQueue*()` pages) and
//! QNX's mqueue server and libc as they use it:
//! - A queue manager (QNX's `mqueue`, which owns `/dev/mqueue`) answers a
//!   client's `mq_open()` with `MsgQueueOpen(rcvid, queueid, msgsize,
//!   maxmsg, oflag)`: the client's connection (the one its message came
//!   on, its future `mqd_t`) is bound to queue `queueid`, or to a new
//!   queue if it is -1 (the result is then its ID, else 0). EBUSY if the
//!   connection is bound already, ESRCH for no such queue or client.
//!   As procnto (measured): the binding is the server connection's, so
//!   shared by a client's connections to the manager's channel but
//!   `_NTO_COF_NOSHARE` ones; a queue's ID is the lowest free slot of the
//!   manager's channel-ID space (a channel made later skips it), and IDs
//!   name only the manager's own queues.
//!   `oflag` holds `_IO_FLAG_RD`/`_IO_FLAG_WR` and `O_NONBLOCK`.
//! - `MsgQueueSend(fd, msg, len, prio)` queues a copy (EMSGSIZE above the
//!   queue's message size, EINVAL for a priority from `MQ_PRIO_MAX` on);
//!   `MsgQueueReceive(fd, buf, len, &prio)` takes the oldest message of
//!   the highest priority (EMSGSIZE if `len` is below the message size)
//!   and returns its length. A full queue blocks a sender
//!   (`STATE_MQ_SEND`), an empty one a receiver (`STATE_MQ_RECEIVE`),
//!   unless the binding is `O_NONBLOCK` (EAGAIN); a `TimerTimeout()`
//!   ends the wait (ETIMEDOUT), a signal too (EINTR). EBADF for a
//!   connection not bound, or not open for that direction.
//! - `MsgQueueCtl(fd, cmd, data)`: `_NTO_MQCTL_GETATTR`/`SETATTR`
//!   (`struct mq_attr`, longs; only `O_NONBLOCK` of `mq_flags` changes),
//!   `_NTO_MQCTL_NOTIFY` (a `sigevent` delivered to the registering
//!   process when a message arrives at an empty queue nobody waits on,
//!   then forgotten; NULL unregisters; EBUSY if another process is
//!   registered), `_NTO_MQCTL_GETBUFSIZE` (fd -1: the memory a queue of
//!   `maxmsg` messages of `msgsize` bytes takes).
//! - `MsgQueueClose(rcvid, queueid)` unbinds the client's connection
//!   (`mq_close()`), or with rcvid -1 drops the queue's creation reference
//!   (`mq_unlink()`): it is then no longer found by ID, its ID is free,
//!   and it goes once no connection is bound to it. EINVAL for an ID that
//!   isn't one of the caller's queues. A manager's queues lose their
//!   creation references when it ends.
//!
//! A binding lives with its server connection (`channel::Scoid::mq`):
//! the client's last connection there going unbinds it. Connections a
//! fork copies are not bound (the child has its own server connection):
//! the child's loader tells the manager (`_IO_DUP`), which binds them.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EBADF, EBUSY, EFAULT, EINVAL, ENOSYS, ESRCH};
use super::event::{self, SigEvent};
use super::process::{self, ProcessRef};
use super::umem::{self, Space};
use crate::sched::{self, State, ThreadRef};
use crate::sync::IrqSpinLock;

use crate::ker::errno::EMSGSIZE;
/// `MQ_PRIO_MAX`: priorities are 0 to 31.
const PRIO_MAX: u32 = 32;
const IO_FLAG_RD: u32 = 1;
const IO_FLAG_WR: u32 = 2;
/// FreeBSD's, the kernel's; a QNX caller's is converted.
const O_NONBLOCK: u32 = proto::fcntl::O_NONBLOCK;

/// `O_NONBLOCK` in the caller's numbers, in the kernel's.
fn nonblock_in(flags: u32) -> u32 {
    if process::current().personality() == process::Personality::Qnx {
        qnxabi::fcntl::flags_from_qnx(flags & !3) & O_NONBLOCK
    } else {
        flags & O_NONBLOCK
    }
}

/// The kernel's `O_NONBLOCK`, in the caller's numbers.
fn nonblock_out(flags: u32) -> u32 {
    if process::current().personality() == process::Personality::Qnx {
        qnxabi::fcntl::flags_to_qnx(flags & O_NONBLOCK)
    } else {
        flags & O_NONBLOCK
    }
}
/// `SI_MESGQ`: a notification's `si_code`.
const SI_MESGQ: i32 = -5;

const MQCTL_NOTIFY: u32 = 1;
const MQCTL_GETATTR: u32 = 2;
const MQCTL_SETATTR: u32 = 3;
const MQCTL_GETBUFSIZE: u32 = 4;
/// Kernel memory per message beyond its bytes (for `GETBUFSIZE`).
const MSG_OVERHEAD: u64 = 32;

struct Message {
    prio: u32,
    data: Vec<u8>,
}

struct QueueState {
    /// By priority, oldest first.
    msgs: BTreeMap<u32, VecDeque<Message>>,
    count: u32,
    senders: VecDeque<ThreadRef>,
    receivers: VecDeque<ThreadRef>,
    /// The registered notification, and its process.
    notify: Option<(ProcessRef, SigEvent)>,
}

pub struct Queue {
    id: i32,
    msgsize: u32,
    maxmsg: u32,
    state: IrqSpinLock<QueueState>,
}

/// A connection's binding to a queue (`Connection::mq`).
pub struct Binding {
    queue: Arc<Queue>,
    /// `_IO_FLAG_RD`, `_IO_FLAG_WR`, `O_NONBLOCK`.
    flags: AtomicU32,
}

/// The queues with their creation reference, by (manager pid, ID).
static QUEUES: IrqSpinLock<BTreeMap<(i32, i32), Arc<Queue>>> = IrqSpinLock::new(BTreeMap::new());
/// The queue each blocked thread waits on, by tid (for `unblock`).
static BLOCKED: IrqSpinLock<BTreeMap<u32, Arc<Queue>>> = IrqSpinLock::new(BTreeMap::new());

/// The client (process, connection) of a REPLY-blocked `rcvid` on one of
/// the caller's channels.
fn client(rcvid: i64) -> Result<(ProcessRef, i32), u32> {
    let coid = super::msg::client_coid(rcvid)?;
    let t = u32::try_from(rcvid)
        .ok()
        .and_then(sched::thread)
        .ok_or(ESRCH)?;
    Ok((t.process.arc(), coid))
}

/// `MsgQueueOpen(rcvid, queueid, msgsize, maxmsg, oflag)`.
/// As procnto's `ker_msg_queue_open`: a queue ID below -1 is EINVAL, and
/// making a queue takes the caller's `PROCMGR_AID_MSG_QUEUE`, both before
/// the client is looked up.
pub fn msg_queue_open(rcvid: i64, queueid: i32, msgsize: u32, maxmsg: u32, oflag: u32) -> KerResult {
    if queueid < -1 {
        return Err(EINVAL);
    }
    if queueid == -1 {
        super::ability::check(super::ability::MSG_QUEUE, None)?;
    }
    let (p, coid) = client(rcvid)?;
    let conn = p.connection(coid).ok_or(ESRCH)?;
    if super::channel::with_queue_binding(&conn, |b| b.is_some()) {
        return Err(EBUSY);
    }
    let me = process::current();
    let (queue, result) = if queueid == -1 {
        if msgsize == 0 || maxmsg == 0 {
            return Err(EINVAL);
        }
        let id = me.add_queue_id();
        let q = Arc::new(Queue {
            id,
            msgsize,
            maxmsg,
            state: IrqSpinLock::new(QueueState {
                msgs: BTreeMap::new(),
                count: 0,
                senders: VecDeque::new(),
                receivers: VecDeque::new(),
                notify: None,
            }),
        });
        QUEUES.lock().insert((me.pid, id), q.clone());
        (q, id as u64)
    } else {
        let q = QUEUES.lock().get(&(me.pid, queueid)).cloned().ok_or(ESRCH)?;
        (q, 0)
    };
    let binding = Binding {
        queue,
        flags: AtomicU32::new((oflag & (IO_FLAG_RD | IO_FLAG_WR)) | nonblock_in(oflag)),
    };
    // Bound only if still free: another of the manager's threads may have
    // bound this server connection since the check above. A queue made
    // for this open then goes again, its ID with it.
    let bound = super::channel::with_queue_binding(&conn, |b| match b {
        Some(_) => Err(binding),
        None => {
            *b = Some(binding);
            Ok(())
        }
    });
    if let Err(binding) = bound {
        if queueid == -1 {
            QUEUES.lock().remove(&(me.pid, binding.queue.id));
            me.remove_queue_id(binding.queue.id);
        }
        return Err(EBUSY);
    }
    Ok(result)
}

/// The calling process's binding of connection `fd`: (queue, flags).
fn binding(fd: i32) -> Result<(Arc<Queue>, u32), u32> {
    let conn = process::current().connection(fd).ok_or(EBADF)?;
    super::channel::with_queue_binding(&conn, |b| {
        let b = b.as_ref().ok_or(EBADF)?;
        Ok((b.queue.clone(), b.flags.load(Ordering::Relaxed)))
    })
}

/// Block the calling thread on `q` in `state` (its lock passed in, the
/// thread already in the right waiter list); returns the wait's error.
fn wait(q: &Arc<Queue>, st: crate::sync::IrqSpinGuard<'_, QueueState>, state: State) -> Result<(), u32> {
    let me = sched::current();
    BLOCKED.lock().insert(me.tid, q.clone());
    sched::prepare_block(state);
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(state);
    drop(st);
    sched::block();
    crate::timeout::disarm(t);
    BLOCKED.lock().remove(&me.tid);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Ok(()),
        e => Err(e),
    }
}

/// `MsgQueueSend(fd, msg, len, prio)`.
pub fn msg_queue_send(fd: i32, msg: u64, len: u64, prio: u32) -> KerResult {
    let (q, flags) = binding(fd)?;
    if flags & IO_FLAG_WR == 0 {
        return Err(EBADF);
    }
    if len > q.msgsize as u64 {
        return Err(EMSGSIZE);
    }
    if prio >= PRIO_MAX {
        return Err(EINVAL);
    }
    let mut data = alloc::vec![0u8; len as usize];
    umem::read(Space::current(), msg, &mut data).map_err(|_| EFAULT)?;
    let me = sched::current();
    loop {
        let mut st = q.state.lock();
        if st.count < q.maxmsg {
            let was_empty = st.count == 0;
            st.msgs.entry(prio).or_default().push_back(Message { prio, data });
            st.count += 1;
            let receiver = st.receivers.pop_front();
            let notify = if was_empty && receiver.is_none() {
                st.notify.take()
            } else {
                None
            };
            drop(st);
            if let Some(r) = receiver {
                sched::wake(&r);
            }
            if let Some((p, ev)) = notify {
                let _ = event::deliver(&p, None, &ev, SI_MESGQ);
            }
            return Ok(0);
        }
        if flags & O_NONBLOCK != 0 {
            return Err(EAGAIN);
        }
        st.senders.push_back(me.arc());
        wait(&q, st, State::MqSend)?;
    }
}

/// `MsgQueueReceive(fd, buf, len, prio)`: returns the message's length.
pub fn msg_queue_receive(fd: i32, buf: u64, len: u64, prio: u64) -> KerResult {
    let (q, flags) = binding(fd)?;
    if flags & IO_FLAG_RD == 0 {
        return Err(EBADF);
    }
    if len < q.msgsize as u64 {
        return Err(EMSGSIZE);
    }
    let me = sched::current();
    let m = loop {
        let mut st = q.state.lock();
        if let Some(mut entry) = st.msgs.last_entry() {
            let m = entry.get_mut().pop_front().unwrap();
            if entry.get().is_empty() {
                entry.remove();
            }
            st.count -= 1;
            let sender = st.senders.pop_front();
            drop(st);
            if let Some(s) = sender {
                sched::wake(&s);
            }
            break m;
        }
        if flags & O_NONBLOCK != 0 {
            return Err(EAGAIN);
        }
        st.receivers.push_back(me.arc());
        wait(&q, st, State::MqReceive)?;
    };
    // A fault loses the message (as QNX documents).
    let space = Space::current();
    umem::write(space, buf, &m.data).map_err(|_| EFAULT)?;
    if prio != 0 {
        umem::write(space, prio, &m.prio.to_le_bytes()).map_err(|_| EFAULT)?;
    }
    Ok(m.data.len() as u64)
}

/// `MsgQueueCtl(fd, cmd, data)`.
pub fn msg_queue_ctl(fd: i32, cmd: u32, data: u64) -> KerResult {
    let space = Space::current();
    if cmd == MQCTL_GETBUFSIZE {
        let mut b = [0u8; 8];
        umem::read(space, data, &mut b)?;
        let maxmsg = u32::from_le_bytes(b[..4].try_into().unwrap()) as u64;
        let msgsize = u32::from_le_bytes(b[4..].try_into().unwrap()) as u64;
        let size = maxmsg * (msgsize + MSG_OVERHEAD) + core::mem::size_of::<Queue>() as u64;
        umem::write(space, data, &size.to_le_bytes())?;
        return Ok(0);
    }
    let (q, flags) = binding(fd)?;
    match cmd {
        MQCTL_GETATTR => {
            let st = q.state.lock();
            let attr: [i64; 6] = [
                q.maxmsg as i64,
                q.msgsize as i64,
                nonblock_out(flags) as i64,
                st.count as i64,
                st.senders.len() as i64,
                st.receivers.len() as i64,
            ];
            drop(st);
            let mut out = [0u8; 48];
            for (i, v) in attr.iter().enumerate() {
                out[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
            }
            umem::write(space, data, &out)?;
            Ok(0)
        }
        MQCTL_SETATTR => {
            let mut f = [0u8; 8];
            umem::read(space, data + 16, &mut f)?;
            let nonblock = nonblock_in(i64::from_le_bytes(f) as u32);
            let conn = process::current().connection(fd).ok_or(EBADF)?;
            super::channel::with_queue_binding(&conn, |b| {
                let b = b.as_ref().ok_or(EBADF)?;
                let flags = b.flags.load(Ordering::Relaxed);
                b.flags.store((flags & !O_NONBLOCK) | nonblock, Ordering::Relaxed);
                Ok(0)
            })
        }
        MQCTL_NOTIFY => {
            let me = process::current();
            let mut st = q.state.lock();
            let mine = st
                .notify
                .as_ref()
                .map(|(p, _)| p.pid == me.pid);
            if data == 0 {
                return match mine {
                    Some(true) => {
                        st.notify = None;
                        Ok(0)
                    }
                    Some(false) => Err(EINVAL),
                    None => Ok(0),
                };
            }
            if mine == Some(false) {
                return Err(EBUSY);
            }
            // The event is read with the lock dropped (user memory may
            // fault), so the registration is checked again before it is
            // stored: another process may have registered meanwhile.
            drop(st);
            let ev = event::read(space, data)?;
            let mut st = q.state.lock();
            if st.notify.as_ref().is_some_and(|(p, _)| p.pid != me.pid) {
                return Err(EBUSY);
            }
            st.notify = Some((me.arc(), ev));
            Ok(0)
        }
        _ => Err(ENOSYS),
    }
}

/// `MsgQueueClose(rcvid, queueid)`.
pub fn msg_queue_close(rcvid: i64, queueid: i32) -> KerResult {
    if rcvid == -1 {
        let me = process::current();
        let q = QUEUES.lock().remove(&(me.pid, queueid)).ok_or(EINVAL)?;
        me.remove_queue_id(queueid);
        drop(q);
        return Ok(0);
    }
    let (p, coid) = client(rcvid).map_err(|_| EBADF)?;
    let conn = p.connection(coid).ok_or(EBADF)?;
    let b = super::channel::with_queue_binding(&conn, |b| match b {
        Some(x) if x.queue.id == queueid => Ok(b.take().unwrap()),
        Some(_) => Err(EINVAL),
        None => Err(EBADF),
    })?;
    // Its threads blocked on the queue through this connection: the
    // queue's waiters of this process.
    let gone: Vec<ThreadRef> = {
        let mut guard = b.queue.state.lock();
        let st = &mut *guard;
        let mut out = Vec::new();
        for list in [&mut st.senders, &mut st.receivers] {
            list.retain(|t| {
                let keep = t.process.pid != p.pid;
                if !keep {
                    out.push(t.clone());
                }
                keep
            });
        }
        out
    };
    for t in gone {
        t.wake_error.store(EBADF, Ordering::Relaxed);
        sched::wake(&t);
    }
    Ok(0)
}

/// Take thread `t`, blocked sending to or receiving from a queue, off its
/// list and wake it with `err`. Returns whether it was waiting.
pub fn unblock(t: &sched::Thread, err: u32) -> bool {
    let Some(q) = BLOCKED.lock().get(&t.tid).cloned() else {
        return false;
    };
    let found = {
        let mut guard = q.state.lock();
        let st = &mut *guard;
        let mut found = false;
        for list in [&mut st.senders, &mut st.receivers] {
            if let Some(i) = list.iter().position(|w| core::ptr::eq(&**w, t)) {
                list.remove(i);
                found = true;
            }
        }
        found
    };
    if found {
        t.wake_error.store(err, Ordering::Relaxed);
        sched::wake(t);
    }
    found
}

/// Process `pid` has ended: the queues it made as a manager lose their
/// creation references.
pub fn process_exited(pid: i32) {
    let gone: Vec<Arc<Queue>> = {
        let mut queues = QUEUES.lock();
        let ids: Vec<(i32, i32)> = queues.range((pid, i32::MIN)..=(pid, i32::MAX)).map(|(k, _)| *k).collect();
        ids.iter().filter_map(|k| queues.remove(k)).collect()
    };
    drop(gone);
}

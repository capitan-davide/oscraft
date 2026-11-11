//! Message passing: `MsgSendv`, `MsgReceivev`, `MsgReplyv`, `MsgError`,
//! `MsgSendPulse`.
//!
//! A client sending on a connection either hands its message straight to a
//! RECEIVE-blocked server thread (and becomes REPLY-blocked) or queues on
//! the channel SEND-blocked. A server receiving takes the most important of
//! the queued pulses and senders (priority order), or blocks. The data are
//! copied by whichever thread completes the rendezvous, directly between
//! the two threads' buffers (`umem::copy`, which reaches any process's
//! memory); the blocked thread's buffer descriptions are kept in its
//! `ThreadIpc`.
//!
//! Priority inheritance (System Architecture, "Priority inheritance and
//! messages"): the receiving thread takes the sender's (or pulse's)
//! priority, up or down, and keeps it after replying; if no thread is
//! RECEIVE-blocked, the thread that last received on the channel is boosted
//! when a higher-priority message or pulse arrives. Channels created with
//! `_NTO_CHF_FIXED_PRIORITY` do neither.
//!
//! A buffer that cannot be accessed fails the call of the thread it belongs
//! to with EFAULT. On a send, the server carries on waiting and the send
//! is not consumed. On a receive, the client whose message was being
//! received fails too, with ESRVRFAULT, and a pulse being received is
//! lost (procnto, measured, review IPC-4). Buffers are checked against
//! the address space as a whole, their ends computed without wrapping:
//! a client's send buffers at `MsgSendv()`, a server's receive buffers as
//! a message arrives, the first of a client's reply buffers as the reply
//! comes (`umem::Buf::bounded`).

use alloc::sync::Arc;
use core::cell::UnsafeCell;

use super::channel::{CHF_FIXED_PRIORITY, Channel, ChannelState, Pulse};
use super::errno::*;
use super::umem::{self, Buf, Space};
use super::{KerResult, process};
use crate::sched::{self, State, Thread};

/// `sizeof(struct _pulse)` and `sizeof(struct _msg_info64)`.
const PULSE_SIZE: usize = 24;
const MSG_INFO_SIZE: usize = 64;
/// `_NTO_MI_UNBLOCK_REQ`, `_NTO_MI_BITS_64` (`struct _msg_info.flags`).
const MI_UNBLOCK_REQ: u16 = 0x100;
const MI_BITS_64: u16 = 0x1000;

struct IpcState {
    send: Buf,
    reply: Buf,
    recv: Buf,
    /// `struct _msg_info *` of a pending receive (0: none), in `recv.space`.
    info: u64,
    coid: i32,
    scoid: i32,
    /// The connection a send is blocked on, held until the send returns:
    /// `ConnectDetach()` of it unblocks the send (EBADF, or the server's
    /// unblock pulse with `_NTO_CHF_UNBLOCK`), and the connection itself,
    /// with its scoid and the server's disconnect pulse, goes once the
    /// send is over (procnto, measured: review IPC-6).
    conn: Option<Arc<super::channel::Connection>>,
    /// How much of the message the server received (`MsgInfo()`).
    received: u64,
    /// What the blocked kernel call returns once the thread is woken.
    result: KerResult,
    /// How far a reply reached in `reply` (`MsgForwardv()` says so).
    replied: u64,
    /// Counts this thread's sends: a long `MsgWrite()` checks between
    /// pieces that it still writes into the same message's reply.
    seq: u64,
}

/// A thread's message-passing state.
///
/// The fields in `state` are written by the thread itself before it blocks
/// and, while it is blocked on a channel, by other threads holding that
/// channel's lock (which also owns the thread's place in the channel's
/// queues).
pub struct ThreadIpc {
    /// Channel this thread is SEND-, RECEIVE- or REPLY-blocked on.
    blocked_on: crate::sync::IrqSpinLock<Option<Arc<Channel>>>,
    /// The client this thread forwards a message for (`msg_forwardv`):
    /// who the server sees.
    acting_for: crate::sync::IrqSpinLock<Option<sched::ThreadRef>>,
    state: UnsafeCell<IpcState>,
}

// SAFETY: see the access rules above.
unsafe impl Sync for ThreadIpc {}

impl ThreadIpc {
    pub const fn new() -> Self {
        Self {
            blocked_on: crate::sync::IrqSpinLock::new(None),
            acting_for: crate::sync::IrqSpinLock::new(None),
            state: UnsafeCell::new(IpcState {
                send: Buf::empty(),
                reply: Buf::empty(),
                recv: Buf::empty(),
                info: 0,
                coid: 0,
                scoid: 0,
                conn: None,
                received: 0,
                result: Ok(0),
                replied: 0,
                seq: 0,
            }),
        }
    }

    /// # Safety
    ///
    /// Only for the thread itself, or for a thread holding the lock of the
    /// channel this thread is blocked on (and with it queued there).
    #[allow(clippy::mut_from_ref)]
    unsafe fn get(&self) -> &mut IpcState {
        // SAFETY: forwarded from the caller.
        unsafe { &mut *self.state.get() }
    }

    /// The client it forwards for, whom its server sees.
    pub fn client(&self) -> Option<sched::ThreadRef> {
        self.acting_for.lock().clone()
    }

    fn set_blocked_on(&self, ch: Option<Arc<Channel>>) {
        *self.blocked_on.lock() = ch;
    }

    /// The channel the thread is SEND-, RECEIVE- or REPLY-blocked on.
    pub fn blocked_on(&self) -> Option<Arc<Channel>> {
        self.blocked_on.lock().clone()
    }
}

/// Which side of a transfer could not be accessed.
enum Fault {
    Server,
    Client,
}

// ANCHOR: transfer
/// Pass `client`'s message to `server`; returns the receive ID. Called
/// with the channel unlocked, once the caller has taken the other thread
/// off the channel's queues (the server off `receivers`, or the client off
/// `senders`): neither can be reached by anyone else meanwhile, so the
/// copy may take as long as it needs and let higher-priority threads run
/// (`umem::copy_preemptible`, review IPC-10).
fn transfer(ch: &Channel, server: &Thread, client: &Thread) -> Result<u64, Fault> {
    // SAFETY: one of the two is the caller, the other blocked and off the
    // channel's queues, so this is the only access to their state.
    let s = unsafe { server.ipc.get() };
    // SAFETY: as above.
    let c = unsafe { client.ipc.get() };
    // The server's buffers must lie within its address space, used or
    // not (procnto checks them as the message arrives, measured).
    if !s.recv.bounded() {
        return Err(Fault::Server);
    }
    // The client's memory unreadable: its send fails. The server's
    // unwritable where the message reaches it: its receive fails (and the
    // client's send, ESRVRFAULT; measured, no part of the message is kept).
    let msglen = umem::copy_preemptible(&s.recv, &c.send).map_err(|side| match side {
        umem::Side::Src => Fault::Client,
        umem::Side::Dst => Fault::Server,
    })?;
    c.received = msglen;
    if s.info != 0 {
        let info = msg_info(ch, client, c, msglen);
        umem::write(s.recv.space, s.info, &info).map_err(|_| Fault::Server)?;
    }
    if ch.flags & CHF_FIXED_PRIORITY == 0 {
        inherit(server, client.priority());
    }
    comm(COMM_RMSG, client.tid, ch.owner.pid);
    Ok(client.tid as u64)
}
// ANCHOR_END: transfer

// Communication events (`_NTO_TRACE_COMM_*`)
const COMM_SMSG: u32 = 0;
const COMM_SPULSE: u32 = 1;
const COMM_RMSG: u32 = 2;
const COMM_RPULSE: u32 = 3;
const COMM_REPLY: u32 = 10;
const COMM_ERROR: u32 = 11;

/// Trace a communication event: a receive ID, scoid or thread ID, and a
/// pid (the server's for messages and pulses, the client's for replies).
fn comm(event: u32, id: u32, pid: i32) {
    super::trace::emit(super::trace::C_COMM, event, &[id, pid as u32]);
}

/// Write `pulse` into `server`'s receive buffer, with priority inheritance.
fn deliver_pulse(ch: &Channel, server: &Thread, p: Pulse) -> Result<(), u32> {
    let mut b = [0u8; PULSE_SIZE];
    // type = _PULSE_TYPE (0), subtype = _PULSE_SUBTYPE (0)
    b[4] = p.code as u8;
    b[8..16].copy_from_slice(&p.value.to_le_bytes());
    b[16..20].copy_from_slice(&p.scoid.to_le_bytes());
    // SAFETY: as in `transfer`.
    let recv = unsafe { &server.ipc.get().recv };
    // A buffer shorter than a pulse takes none of it: the receive fails
    // with EFAULT and the pulse is lost, as on procnto (measured; review
    // IPC-9: it was delivered cut short).
    if !recv.bounded() || recv.total() < PULSE_SIZE as u64 {
        return Err(EFAULT);
    }
    umem::copy(recv, &Buf::kernel(&b))?;
    if ch.flags & CHF_FIXED_PRIORITY == 0 {
        inherit(server, p.priority);
    }
    comm(COMM_RPULSE, p.scoid as u32, ch.owner.pid);
    Ok(())
}

/// The receiving thread `server` takes the priority of what it received
/// (a client's, a pulse's) as its own, as procnto (measured): its base
/// priority, which `pthread_getschedparam()` and `SchedGet()` report,
/// and keeps after the reply until its next receive. A mutex that raises
/// it keeps it above that (`sched::set_base_priority`). A sporadic server
/// only runs at it: its own priority is its budget's (not measured). It is
/// capped at the highest the server's process may take (63 without
/// `PROCMGR_AID_PRIORITY`, procnto, measured). It takes the mutex-raise
/// table (`sync::RAISED`), sometimes with the channel's lock held (a
/// pulse's delivery): nothing may take a channel's lock while holding that
/// table.
fn inherit(server: &Thread, prio: u8) {
    let prio = prio.min(super::ability::priority_cap(&server.process));
    if server.policy() == sched::Policy::Sporadic {
        sched::set_priority(server, prio);
    } else {
        sched::set_base_priority(server, prio);
    }
}

// ANCHOR: boost
/// Server boost: nobody is RECEIVE-blocked, so raise the thread that last
/// received on the channel to the waiting work's priority, capped as a
/// receive is (`inherit`; procnto, measured).
fn boost(ch: &Channel, st: &ChannelState, prio: u8) {
    if ch.flags & CHF_FIXED_PRIORITY != 0 {
        return;
    }
    if let Some(r) = st.last_receiver.as_ref() {
        let prio = prio.min(super::ability::priority_cap(&r.process));
        if r.priority() < prio {
            sched::set_priority(r, prio);
        }
    }
}
// ANCHOR_END: boost

/// Release a thread blocked on a channel with an error. Call with that
/// channel's lock held, after unlinking the thread.
pub fn fail_blocked(t: &Thread, err: u32) {
    // SAFETY: the caller holds the lock of the channel `t` is blocked on.
    unsafe { t.ipc.get() }.result = Err(err);
    sched::wake(t);
}

/// Whether two processes have the same effective user ID: their
/// credential locks are taken one after the other, never both at once
/// (two processes pulsing each other from two CPUs would otherwise each
/// hold one and spin on the other, IRQs masked: review item IPC-5).
fn same_user(a: &process::Process, b: &process::Process) -> bool {
    let ea = a.cred.lock().euid;
    let eb = b.cred.lock().euid;
    ea == eb
}

/// Block the calling thread on `ch` in `state` (channel lock passed in) and
/// return its kernel call's result once woken.
fn block_on(
    ch: &Arc<Channel>,
    st: crate::sync::IrqSpinGuard<'_, ChannelState>,
    state: State,
) -> KerResult {
    let me = sched::current();
    me.ipc.set_blocked_on(Some(ch.clone()));
    sched::prepare_block(state);
    // A send's timeout covers the reply wait that follows (`arm_send`).
    let t = if state == State::Send {
        crate::timeout::arm_send()
    } else {
        crate::timeout::arm(state)
    };
    drop(st);
    sched::block();
    crate::timeout::disarm(t);
    me.ipc.set_blocked_on(None);
    // SAFETY: we are running again and no longer queued on the channel.
    unsafe { me.ipc.get() }.result
}

/// `MsgSendv(coid, smsg, sparts, rmsg, rparts)`: returns the reply status.
pub fn msg_sendv(coid: i32, smsg: u64, sparts: i64, rmsg: u64, rparts: i64) -> KerResult {
    let conn = process::current().connection(coid).ok_or(EBADF)?;
    let ch = conn.channel.clone();
    let me = sched::current();
    let space = Space::current();
    let (send, reply) = (
        umem::parse(space, smsg, sparts)?,
        umem::parse(space, rmsg, rparts)?,
    );
    // The message's buffers must lie within the address space, each
    // starting in readable memory, whether or not the server takes them;
    // the reply buffers are checked as the reply comes (procnto, measured,
    // review IPC-4).
    if !send.bounded() || !send.heads_readable() {
        return Err(EFAULT);
    }
    // A connection bound to a kernel pipe: its reads and writes are the
    // kernel's (`kpipe`).
    if let Some(r) = super::kpipe::msg_send(&conn, &send, &reply) {
        return r;
    }
    {
        // SAFETY: our own state, before we become visible on the channel.
        let s = unsafe { me.ipc.get() };
        (s.send, s.reply, s.coid, s.scoid, s.result) = (send, reply, coid, conn.scoid, Ok(0));
        s.conn = Some(conn.clone());
        s.seq = s.seq.wrapping_add(1);
    }
    drop(conn);
    let r = match qnxd_for(&ch, me) {
        Ok(via) => send_on(via.as_ref().unwrap_or(&ch), me),
        Err(e) => Err(e),
    };
    // SAFETY: running again, no longer queued on the channel.
    unsafe { me.ipc.get() }.conn = None;
    r
}

/// The channel of `qnxd`, the QNX personality's service (`pathmgr`'s `_FTYPE_QNX`).
static QNXD: crate::sync::IrqSpinLock<Option<Arc<Channel>>> = crate::sync::IrqSpinLock::new(None);

// ANCHOR: qnxd_for
/// Where `me`'s message on `ch` goes first, its send state set: `qnxd`,
/// for a QNX program's message to a native server, which speaks FreeBSD.
/// Without `qnxd` that send fails: the two can't understand each other,
/// and a QNX program must never read a FreeBSD reply as its own. Plain
/// reads and writes have nothing to translate and go straight to the
/// server; process 1 answers each process itself.
fn qnxd_for(ch: &Channel, me: &Thread) -> Result<Option<Arc<Channel>>, u32> {
    const IO_READ: u16 = 0x101;
    const IO_WRITE: u16 = 0x102;
    const IO_NOTIFY: u16 = 0x105;
    if me.process.personality() != process::Personality::Qnx || ch.owner.pid == process::KERNEL_PID {
        return Ok(None);
    }
    let native = process::lookup(ch.owner.pid).is_some_and(|p| p.personality() == process::Personality::Native);
    if !native {
        return Ok(None);
    }
    let qnxd = QNXD.lock().clone();
    if let Some(q) = &qnxd
        && q.owner.pid == ch.owner.pid
    {
        return Ok(None);
    }
    let mut h = [0u8; 9];
    // SAFETY: our own state, set by the send.
    let n = umem::copy(&umem::Buf::kernel_mut(&mut h), &unsafe { me.ipc.get() }.send).unwrap_or(0);
    let ty = u16::from_le_bytes([h[0], h[1]]);
    if n >= 9 && matches!(ty, IO_READ | IO_WRITE) && h[8] == 0 {
        return Ok(None);
    }
    // Nothing in it to translate, and the server must see the client: its
    // event is delivered later, with `rcvid`, when `qnxd` has long gone.
    if n >= 2 && ty == IO_NOTIFY {
        return Ok(None);
    }
    match qnxd {
        Some(q) if !q.state.lock().destroyed => Ok(Some(q)),
        _ => Err(super::errno::ENOSYS),
    }
}
// ANCHOR_END: qnxd_for

/// `qnxd` attached itself to the path manager with `_FTYPE_QNX`: channel
/// `chid` of process `pid` serves the QNX personality. It goes with its
/// channel. Root's alone.
pub fn set_qnxd(pid: i32, chid: i32) -> Result<(), u32> {
    let p = process::lookup(pid).ok_or(ESRCH)?;
    if p.cred.lock().euid != 0 {
        return Err(super::errno::EPERM);
    }
    *QNXD.lock() = Some(p.channel(chid).ok_or(ESRCH)?);
    Ok(())
}

/// `MsgForwardv(rcvid, smsg, sparts, rmsg, rparts, replied)`: `MsgSendv()`
/// for the client of `rcvid`, a message `qnxd` received, on the connection
/// the client sent on: `smsg` goes to that server, which sees the client
/// as its sender, and the calling thread blocks for the reply, into
/// `rmsg`, of which `*replied` bytes. The client stays blocked on `qnxd`,
/// which replies to it.
pub fn msg_forwardv(rcvid: i64, smsg: u64, sparts: i64, rmsg: u64, rparts: i64, replied: u64) -> KerResult {
    let client = u32::try_from(rcvid).ok().and_then(sched::thread).ok_or(ESRCH)?;
    let on = client.ipc.blocked_on().ok_or(ESRCH)?;
    let is_qnxd = QNXD.lock().as_ref().is_some_and(|t| Arc::ptr_eq(t, &on));
    if !is_qnxd || on.owner.pid != process::current().pid {
        return Err(ESRCH);
    }
    let me = sched::current();
    let space = Space::current();
    let (send, reply) = (umem::parse(space, smsg, sparts)?, umem::parse(space, rmsg, rparts)?);
    // Its connection, while it is ours to read: REPLY-blocked on our
    // channel, under the channel's lock.
    let (conn, coid) = {
        let st = on.state.lock();
        if !st.repliers.iter().any(|r| core::ptr::eq(&**r, &*client)) {
            return Err(ESRCH);
        }
        // SAFETY: as above.
        let c = unsafe { client.ipc.get() };
        (c.conn.clone().ok_or(ESRCH)?, c.coid)
    };
    let ch = conn.channel.clone();
    // A client that reached `qnxd` itself (its attach shows in
    // `/proc/mount`): there is no server behind it to forward to, and
    // forwarding would come straight back, a thread at a time.
    if Arc::ptr_eq(&ch, &on) {
        return Err(super::errno::ENOSYS);
    }
    {
        // SAFETY: our own state, before we become visible on the channel.
        let s = unsafe { me.ipc.get() };
        (s.send, s.reply, s.coid, s.scoid, s.result, s.replied) = (send, reply, coid, conn.scoid, Ok(0), 0);
        s.conn = Some(conn);
        s.seq = s.seq.wrapping_add(1);
    }
    *me.ipc.acting_for.lock() = Some(client);
    let r = send_on(&ch, me);
    *me.ipc.acting_for.lock() = None;
    // SAFETY: running again, no longer queued on the channel.
    let s = unsafe { me.ipc.get() };
    s.conn = None;
    if replied != 0 {
        umem::write(space, replied, &s.replied.to_le_bytes()).map_err(|_| super::errno::EFAULT)?;
    }
    r
}

/// The send proper, its state set: hand the message to a waiting
/// receiver, or queue and block.
fn send_on(ch: &Arc<Channel>, me: &Thread) -> KerResult {
    me.unblock_requested
        .store(false, core::sync::atomic::Ordering::Relaxed);
    comm(COMM_SMSG, me.tid, ch.owner.pid);
    let mut st = ch.state.lock();
    // Its server gone: the connection no longer leads anywhere (EBADF);
    // ESRCH is for a thread blocked as it went.
    if st.destroyed {
        return Err(EBADF);
    }
    // The first waiting receiver takes the message; a receiver whose
    // buffer can't fails along with the send (procnto, measured). Off the
    // queue, it is ours: the message is copied with the channel unlocked.
    if let Some(server) = st.receivers.pop_front() {
        drop(st);
        let r = transfer(ch, &server, me);
        st = ch.state.lock();
        // Destroyed meanwhile: its destruction couldn't release the
        // server, off the queue; it goes as the others did (ESRCH), and
        // the send fails as on a destroyed channel.
        if st.destroyed {
            fail_blocked(&server, ESRCH);
            return Err(EBADF);
        }
        match r {
            Ok(rcvid) => {
                // SAFETY: `server` was RECEIVE-blocked on this channel; we hold its lock.
                unsafe { server.ipc.get() }.result = Ok(rcvid);
                st.repliers.push(me.arc());
                // Becomes REPLY-blocked before the server can run and reply.
                me.ipc.set_blocked_on(Some(ch.clone()));
                sched::prepare_block(State::Reply);
                let t = crate::timeout::arm(State::Reply);
                sched::wake(&server);
                // SAFETY: our own state; only we write it.
                let scoid = unsafe { me.ipc.get() }.scoid;
                drop(st);
                // Forwarding for a client a signal asked for meanwhile,
                // before there was a server to ask.
                let asked = me.ipc.acting_for.lock().as_ref().is_some_and(|c| c.unblock_requested.load(core::sync::atomic::Ordering::Relaxed));
                if asked && ch.flags & CHF_UNBLOCK != 0 && !me.unblock_requested.swap(true, core::sync::atomic::Ordering::Relaxed) {
                    channel_pulse(ch, me.priority(), PULSE_CODE_UNBLOCK, me.tid as u64, scoid);
                }
                sched::block();
                crate::timeout::disarm(t);
                me.ipc.set_blocked_on(None);
                // SAFETY: running again, no longer queued on the channel.
                return unsafe { me.ipc.get() }.result;
            }
            Err(Fault::Client) => {
                st.receivers.push_front(server);
                return Err(EFAULT);
            }
            // The server's buffer is bad: its receive fails, and so does
            // the send, with ESRVRFAULT (procnto, measured).
            Err(Fault::Server) => {
                fail_blocked(&server, EFAULT);
                return Err(ESRVRFAULT);
            }
        }
    }
    st.add_sender(me);
    boost(ch, &st, me.priority());
    block_on(ch, st, State::Send)
}

/// `ConnectDetach()` of `coid` by its process: a thread of it blocked in
/// a send on that connection leaves as on a signal, with EBADF (procnto,
/// measured, review IPC-6): a SEND-blocked one at once, the server never
/// seeing the message; a REPLY-blocked one at once unless the channel
/// has `_NTO_CHF_UNBLOCK`, when the server gets the unblock pulse and
/// the client waits for its reply.
pub fn connection_detached(p: &process::Process, coid: i32, ch: &Arc<Channel>) {
    for t in sched::threads_of(p) {
        if !matches!(t.state(), State::Send | State::Reply) {
            continue;
        }
        let on_it = t.ipc.blocked_on().is_some_and(|c| Arc::ptr_eq(&c, ch));
        // SAFETY: `coid` is written by the thread before it blocks and
        // read here while it is; a stale value only names another
        // connection, which `on_it` and the state rule out.
        if on_it && unsafe { t.ipc.get() }.coid == coid {
            unblock(&t, EBADF);
        }
    }
}

/// The connection thread `t` waits for a reply on, if it does.
pub fn reply_connection(t: &sched::Thread) -> Option<Arc<super::channel::Connection>> {
    if t.state() != State::Reply {
        return None;
    }
    // SAFETY: `conn` is written by the thread before it blocks and held
    // until its send returns; it is read here while the thread is blocked.
    unsafe { t.ipc.get() }.conn.clone()
}

/// `MsgReceivev(chid, rmsg, rparts, info)`: returns the receive ID, or 0
/// for a pulse.
pub fn msg_receivev(chid: i32, rmsg: u64, rparts: i64, info: u64) -> KerResult {
    receive(chid, rmsg, rparts, info, false)
}

/// `MsgReceivePulsev(chid, rmsg, rparts, info)`: as `MsgReceivev()`, but
/// only a pulse: messages stay for the other receivers. Returns 0.
pub fn msg_receive_pulsev(chid: i32, rmsg: u64, rparts: i64, info: u64) -> KerResult {
    receive(chid, rmsg, rparts, info, true)
}

fn receive(chid: i32, rmsg: u64, rparts: i64, info: u64, pulses_only: bool) -> KerResult {
    let ch = process::current().channel(chid).ok_or(ESRCH)?;
    let me = sched::current();
    let recv = umem::parse(Space::current(), rmsg, rparts)?;
    {
        // SAFETY: our own state, before we become visible on the channel.
        let s = unsafe { me.ipc.get() };
        (s.recv, s.info, s.result) = (recv, info, Ok(0));
    }
    let mut st = ch.state.lock();
    // Destroyed between the lookup and the lock: the threads blocked on
    // it were released with ESRCH, and so are we, instead of joining a
    // queue nothing drains again (review IPC-7).
    if st.destroyed {
        return Err(ESRCH);
    }
    st.last_receiver = Some(me.arc());
    loop {
        let pulse_first = match (st.pulses.front(), st.senders.first()) {
            (Some(_), _) if pulses_only => true,
            (Some(p), Some(c)) => p.priority >= c.priority(),
            (Some(_), None) => true,
            _ => false,
        };
        if pulse_first {
            // A pulse the receive buffer can't take is lost with the
            // receive (procnto, measured).
            let p = st.pulses.pop_front().unwrap();
            deliver_pulse(&ch, me, p)?;
            return Ok(0);
        }
        if st.senders.is_empty() || pulses_only {
            break;
        }
        // Off the queue, the client is ours: its message is copied with
        // the channel unlocked (as in `send_on`).
        let client = st.senders.remove(0);
        drop(st);
        let r = transfer(&ch, me, &client);
        st = ch.state.lock();
        // Destroyed meanwhile: the client goes as the others did, and so
        // do we (ESRCH).
        if st.destroyed {
            fail_blocked(&client, ESRCH);
            return Err(ESRCH);
        }
        match r {
            Ok(rcvid) => {
                sched::set_blocked_state(&client, State::Reply);
                st.repliers.push(client);
                return Ok(rcvid);
            }
            // Our buffer is bad: the client's send fails with ESRVRFAULT
            // (procnto, measured), ours with EFAULT.
            Err(Fault::Server) => {
                fail_blocked(&client, ESRVRFAULT);
                return Err(EFAULT);
            }
            // The client's message is unreadable: its send fails.
            Err(Fault::Client) => fail_blocked(&client, EFAULT),
        }
    }
    if pulses_only {
        st.pulse_receivers.push_back(me.arc());
    } else {
        st.receivers.push_back(me.arc());
    }
    block_on(&ch, st, State::Receive)
}

/// Find the REPLY-blocked client of `rcvid` on one of our channels, take it
/// off the channel and complete its `MsgSend` with `finish`, with the
/// channel unlocked: off the repliers, the client is ours, and a long reply
/// may let higher-priority threads run (review IPC-10).
fn complete(rcvid: i64, event: u32, finish: impl FnOnce(&mut IpcState) -> KerResult) -> KerResult {
    let client = u32::try_from(rcvid)
        .ok()
        .and_then(sched::thread)
        .ok_or(ESRCH)?;
    let ch = client.ipc.blocked_on().ok_or(ESRCH)?;
    if ch.owner.pid != process::current().pid {
        return Err(ESRCH);
    }
    if !ch.state.lock().take_replier(&client) {
        return Err(ESRCH);
    }
    // SAFETY: `client` is REPLY-blocked and off the channel's repliers:
    // nobody else reaches its state.
    let r = finish(unsafe { client.ipc.get() });
    comm(event, client.ptid, client.process.pid);
    sched::wake(&client);
    r
}

/// `MsgReplyv(rcvid, status, smsg, sparts)`.
pub fn msg_replyv(rcvid: i64, status: u64, smsg: u64, sparts: i64) -> KerResult {
    // Our own buffers unreadable (the iov array, or the data it names):
    // the reply fails with ESRVRFAULT, and so does the client's send, which
    // it still ends (procnto, measured, review IPC-8). Other errors (too
    // many parts) leave the client waiting.
    let data = match umem::parse(Space::current(), smsg, sparts) {
        Ok(d) => d,
        Err(EFAULT) => return fail_reply(rcvid),
        Err(e) => return Err(e),
    };
    complete(rcvid, COMM_REPLY, |c| {
        // The first of the client's reply buffers must lie within its
        // address space when anything is replied; the others only where
        // the reply reaches them (procnto, measured, review IPC-4). A bad
        // reply buffer fails the client's send; our reply still ends it.
        if data.total() > 0 && !c.reply.first_bounded() {
            c.result = Err(EFAULT);
            return Ok(0);
        }
        match umem::copy_preemptible(&c.reply, &data) {
            Ok(n) => {
                c.replied = c.replied.max(n);
                c.result = Ok(status);
                Ok(0)
            }
            Err(umem::Side::Dst) => {
                c.result = Err(EFAULT);
                Ok(0)
            }
            Err(umem::Side::Src) => {
                c.result = Err(ESRVRFAULT);
                Err(ESRVRFAULT)
            }
        }
    })
}

/// A reply that faulted in the server's own memory before anything was
/// copied: the client's send ends with ESRVRFAULT, and so does the reply.
fn fail_reply(rcvid: i64) -> KerResult {
    complete(rcvid, COMM_REPLY, |c| {
        c.result = Err(ESRVRFAULT);
        Err(ESRVRFAULT)
    })
}

/// `MsgError(rcvid, error)`: the client's `MsgSend` fails with `error`
/// (or returns 0 for EOK).
pub fn msg_error(rcvid: i64, error: u32) -> KerResult {
    complete(rcvid, COMM_ERROR, |c| {
        c.result = if error == 0 { Ok(0) } else { Err(error) };
        Ok(0)
    })
}

/// `MsgSendPulse(coid, priority, code, value)`; priority -1 means the
/// sender's own.
pub fn msg_send_pulse(coid: i32, priority: i32, code: i32, value: u64) -> KerResult {
    // As procnto: -1 is the sender's own priority, above 255 EINVAL; a
    // negative `coid` is the sender's own channel -coid (scoid 0), ESRCH
    // if it has none (measured; review IPC-13).
    let priority = match priority {
        -1 => sched::current().base_priority(),
        0..=255 => priority as u8,
        _ => return Err(EINVAL),
    };
    let me = process::current();
    let (channel, scoid) = if coid < 0 && coid != i32::MIN {
        (me.channel(-coid).ok_or(ESRCH)?, 0)
    } else {
        let conn = me.connection(coid).ok_or(EBADF)?;
        (conn.channel.clone(), conn.scoid)
    };
    if channel.state.lock().destroyed {
        return Err(EBADF);
    }
    // To another user's process: PROCMGR_AID_CONNECTION; procnto's own
    // channels take pulses from anyone (procnto's
    // nto_pulse_allow_crossproc).
    if channel.owner.pid != process::KERNEL_PID
        && let Some(owner) = process::lookup(channel.owner.pid)
        && !core::ptr::eq(&*owner, me)
        && !same_user(me, &owner)
        && !super::ability::allowed(me, super::ability::CONNECTION, None)
    {
        return Err(super::errno::EPERM);
    }
    channel_pulse(&channel, priority, code, value, scoid);
    Ok(0)
}

// ANCHOR: channel_pulse
/// Queue a pulse on channel `ch` (or hand it to a receiving thread). Its
/// priority, which the receiving thread takes, is capped at the highest
/// the channel's process may take (`ability::priority_cap`), whoever sent
/// it: 63 for an unprivileged process, 253 for root (procnto, measured).
/// For that it looks the owner up in the process table and reads its
/// credentials and abilities, on every pulse (IRQ-safe locks: pulses are
/// also sent from interrupts).
pub fn channel_pulse(ch: &Channel, priority: u8, code: i32, value: u64, scoid: i32) {
    let priority = match ch.owner.pid {
        process::KERNEL_PID => priority,
        pid => process::lookup(pid).map_or(priority, |o| priority.min(super::ability::priority_cap(&o))),
    };
    let p = Pulse {
        priority,
        code: code as i8,
        value,
        scoid,
    };
    comm(COMM_SPULSE, scoid as u32, ch.owner.pid);
    let mut st = ch.state.lock();
    if st.destroyed {
        return; // nobody will receive it
    }
    if let Some(server) = st
        .pulse_receivers
        .pop_front()
        .or_else(|| st.receivers.pop_front())
    {
        match deliver_pulse(ch, &server, p) {
            Ok(()) => {
                // SAFETY: `server` was RECEIVE-blocked on this channel; we hold its lock.
                unsafe { server.ipc.get() }.result = Ok(0);
                sched::wake(&server);
            }
            // The receiver's buffer can't take it: its receive fails and
            // the pulse is lost with it (procnto, measured).
            Err(e) => fail_blocked(&server, e),
        }
        return;
    }
    st.add_pulse(p);
    boost(ch, &st, priority);
}
// ANCHOR_END: channel_pulse

/// Thread `t` is ending: its process's channels made with
/// `_NTO_CHF_THREAD_DEATH` get a `_PULSE_CODE_THREADDEATH` pulse, its
/// thread ID the value (as procnto).
pub fn thread_died(t: &Thread) {
    const CHF_THREAD_DEATH: u32 = 0x4;
    const PULSE_CODE_THREADDEATH: i32 = -34;
    if t.process.pid == process::KERNEL_PID {
        return;
    }
    for ch in t.process.channel_list() {
        if ch.flags & CHF_THREAD_DEATH != 0 {
            channel_pulse(&ch, t.priority(), PULSE_CODE_THREADDEATH, t.ptid as u64, 0);
        }
    }
}

/// `struct _msg_info` for a message of `client` (state `c`) on `ch`, of
/// which `msglen` bytes were received.
fn msg_info(ch: &Channel, sender: &Thread, c: &IpcState, msglen: u64) -> [u8; MSG_INFO_SIZE] {
    let client = sender.ipc.client();
    let client = client.as_deref().unwrap_or(sender);
    let mut info = [0u8; MSG_INFO_SIZE];
    let mut put = |off: usize, bytes: &[u8]| info[off..off + bytes.len()].copy_from_slice(bytes);
    put(8, &client.process.pid.to_le_bytes());
    put(12, &(client.ptid as i32).to_le_bytes());
    put(16, &ch.chid.to_le_bytes());
    put(20, &c.scoid.to_le_bytes());
    put(24, &c.coid.to_le_bytes());
    put(28, &(client.priority() as i16).to_le_bytes());
    // `_NTO_MI_BITS_64`, and `_NTO_MI_UNBLOCK_REQ` once the client asked
    // to be unblocked.
    let mut flags = MI_BITS_64 | client.process.personality().msg_info_flags();
    if client.unblock_requested.load(core::sync::atomic::Ordering::Relaxed) {
        flags |= MI_UNBLOCK_REQ;
    }
    put(30, &flags.to_le_bytes());
    put(32, &(msglen as i64).to_le_bytes());
    put(40, &(c.send.wrapping_total() as i64).to_le_bytes());
    put(48, &(c.reply.wrapping_total() as i64).to_le_bytes());
    info
}

/// `MsgInfo(rcvid, info)`: the `struct _msg_info` of a received message.
pub fn msg_info_call(rcvid: i64, info: u64) -> KerResult {
    let client = u32::try_from(rcvid)
        .ok()
        .and_then(sched::thread)
        .ok_or(ESRCH)?;
    let ch = client.ipc.blocked_on().ok_or(ESRCH)?;
    let b = with_replier(rcvid, |c| Ok(msg_info(&ch, &client, c, c.received)))?;
    umem::write(Space::current(), info, &b)?;
    Ok(0)
}

/// `MsgCurrent(rcvid)`: 0 for a message the caller received and hasn't
/// replied to, else ESRCH (as procnto, which changes no priority here).
pub fn msg_current(rcvid: i64) -> KerResult {
    with_replier(rcvid, |_| Ok(0))
}

/// Run `f` on the IPC state of the client `rcvid` while it stays
/// REPLY-blocked on one of the caller's channels.
fn with_replier<R>(rcvid: i64, f: impl FnOnce(&mut IpcState) -> Result<R, u32>) -> Result<R, u32> {
    let client = u32::try_from(rcvid)
        .ok()
        .and_then(sched::thread)
        .ok_or(ESRCH)?;
    let ch = client.ipc.blocked_on().ok_or(ESRCH)?;
    if ch.owner.pid != process::current().pid {
        return Err(ESRCH);
    }
    let st = ch.state.lock();
    if !st.repliers.iter().any(|t| core::ptr::eq(&**t, &*client)) {
        return Err(ESRCH);
    }
    // SAFETY: `client` is REPLY-blocked on this channel; we hold its lock.
    let r = f(unsafe { client.ipc.get() });
    drop(st);
    r
}

/// The connection ID the REPLY-blocked client of `rcvid` (on one of the
/// caller's channels) sent its message on.
pub fn client_coid(rcvid: i64) -> Result<i32, u32> {
    with_replier(rcvid, |s| Ok(s.coid))
}

/// `MsgReadv(rcvid, rmsg, rparts, offset)`: more of a client's message,
/// from byte `offset` on; returns the bytes read.
pub fn msg_readv(rcvid: i64, rmsg: u64, rparts: i64, offset: u64) -> KerResult {
    let dst = umem::parse(Space::current(), rmsg, rparts)?;
    // Our own buffers past the address space, or faulting: ESRVRFAULT;
    // the client's faulting: EFAULT (procnto, measured and documented).
    if !dst.bounded() {
        return Err(ESRVRFAULT);
    }
    // The client's message as it is now, and its thread held: the copy
    // runs with the channel unlocked and may let higher-priority threads
    // run (review IPC-10). The client may give up meanwhile; holding its
    // thread keeps its process, so the copy reads what is left, or faults.
    let (client, src) = replier_message(rcvid, offset)?;
    let r = umem::copy_preemptible(&dst, &src).map_err(|side| match side {
        umem::Side::Dst => ESRVRFAULT,
        umem::Side::Src => EFAULT,
    });
    drop(client);
    r
}

/// The REPLY-blocked client of `rcvid` (on one of the caller's channels)
/// and its message from byte `offset` on.
fn replier_message(rcvid: i64, offset: u64) -> Result<(sched::ThreadRef, Buf), u32> {
    let client = u32::try_from(rcvid)
        .ok()
        .and_then(sched::thread)
        .ok_or(ESRCH)?;
    let send = with_replier(rcvid, |c| Ok(c.send.skip(offset)))?;
    Ok((client, send))
}

/// `MsgWritev(rcvid, smsg, sparts, offset)`: write into a client's reply
/// buffer at `offset`, before replying; returns the bytes written.
///
/// A long write goes `umem::CHUNK` at a time, each piece under the
/// channel's lock with the client checked still REPLY-blocked on the same
/// message, and a preemption point between pieces (issue #287): not one
/// masked copy, and not unlocked either, since a client that leaves
/// meanwhile (a signal, without `_NTO_CHF_UNBLOCK`) takes its buffer back
/// and must not be written to after. Such a client fails the rest with
/// ESRCH, as its rcvid would.
pub fn msg_writev(rcvid: i64, smsg: u64, sparts: i64, offset: u64) -> KerResult {
    let src = umem::parse(Space::current(), smsg, sparts)?;
    // Our buffers aren't checked as a whole here (procnto writes 64 bytes
    // from a wrapping one, measured); a fault in them is ESRVRFAULT.
    let fault = |side| match side {
        umem::Side::Src => ESRVRFAULT,
        umem::Side::Dst => EFAULT,
    };
    let mut seq = None;
    let mut done = 0;
    loop {
        let n = with_replier(rcvid, |c| {
            if *seq.get_or_insert(c.seq) != c.seq {
                return Err(ESRCH);
            }
            let n = umem::copy(&c.reply.skip(offset.saturating_add(done)).take(umem::CHUNK), &src.skip(done)).map_err(fault)?;
            c.replied = c.replied.max(offset.saturating_add(done).saturating_add(n));
            Ok(n)
        })?;
        done += n;
        if n < umem::CHUNK {
            return Ok(done);
        }
        sched::preemption_point();
    }
}

/// `_NTO_CHF_UNBLOCK`: the server decides when a REPLY-blocked client may
/// leave (it gets `_PULSE_CODE_UNBLOCK` with the rcvid).
const CHF_UNBLOCK: u32 = 0x2;
const PULSE_CODE_UNBLOCK: i32 = -32;

/// Make thread `t`, blocked in message passing, fail its call with `err`
/// (a signal: EINTR; a timeout: ETIMEDOUT). A SEND- or RECEIVE-blocked
/// thread leaves at once; a REPLY-blocked one too, unless its server's
/// channel has `_NTO_CHF_UNBLOCK`: then the server is asked (once) and
/// the client waits for its reply. Returns whether `t` was released.
///
/// A request to procnto waiting to be received stays queued too, marked
/// as asking to leave (`_NTO_MI_UNBLOCK_REQ`), as if procnto had received
/// it: procnto's thread pool receives at once, so on QNX a signal finds
/// such a request REPLY-blocked, and `mmap()` & co. never fail with EINTR
/// (measured: EINTR 3 times in 10000 `mmap()`s under SIGCHLD before).
/// procnto then answers it as any other. A thread that is to end
/// (`exit_on_return`) leaves at once, as procnto may be waiting for it.
pub fn unblock(t: &Thread, err: u32) -> bool {
    use core::sync::atomic::Ordering;
    let Some(ch) = t.ipc.blocked_on() else {
        return false;
    };
    let mut st = ch.state.lock();
    match t.state() {
        State::Send
            if ch.owner.pid == process::KERNEL_PID
                && ch.flags & CHF_UNBLOCK != 0
                && err == EINTR
                && !t.exit_on_return.load(Ordering::Relaxed) =>
        {
            t.unblock_requested.store(true, Ordering::Relaxed);
            false
        }
        State::Send => match st.senders.iter().position(|c| core::ptr::eq(&**c, t)) {
            Some(i) => {
                st.senders.remove(i);
                fail_blocked(t, err);
                true
            }
            None => false,
        },
        State::Receive => {
            let st = &mut *st;
            let queues = [&mut st.receivers, &mut st.pulse_receivers];
            for q in queues {
                if let Some(i) = q.iter().position(|c| core::ptr::eq(&**c, t)) {
                    q.remove(i);
                    fail_blocked(t, err);
                    return true;
                }
            }
            false
        }
        State::Reply if ch.flags & CHF_UNBLOCK != 0 => {
            // Off the repliers, its reply is being copied (`complete`, with
            // the channel unlocked): it ends at once, nothing to ask.
            if !st.repliers.iter().any(|r| core::ptr::eq(&**r, t)) {
                return false;
            }
            if !t.unblock_requested.swap(true, Ordering::Relaxed) {
                // SAFETY: `t` is REPLY-blocked on `ch`; we hold its lock.
                let scoid = unsafe { t.ipc.get() }.scoid;
                drop(st);
                // On `qnxd`: the server has the message, and it is the
                // one to ask, through the thread forwarding it.
                if QNXD.lock().as_ref().is_some_and(|q| Arc::ptr_eq(q, &ch)) {
                    let forwarder = process::lookup(ch.owner.pid).and_then(|p| {
                        sched::threads_of(&p).into_iter().find(|f| {
                            f.ipc.acting_for.lock().as_ref().is_some_and(|c| core::ptr::eq(&**c, t))
                        })
                    });
                    if let Some(f) = forwarder {
                        unblock(&f, err);
                    }
                    return false;
                }
                channel_pulse(&ch, t.priority(), PULSE_CODE_UNBLOCK, t.tid as u64, scoid);
                // A server the server monitor ignores doesn't hold its
                // clients (measured on procnto); others are watched.
                if super::servmon::ignored(ch.owner.pid) {
                    let mut st = ch.state.lock();
                    if st.take_replier(t) {
                        fail_blocked(t, err);
                        return true;
                    }
                    return false;
                }
                super::servmon::watch(t, ch.owner.pid);
            }
            false
        }
        State::Reply => {
            if st.take_replier(t) {
                fail_blocked(t, err);
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

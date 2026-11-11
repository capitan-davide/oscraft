//! Channels and connections, and the kernel calls that manage them
//! (`ChannelCreate`, `ChannelDestroy`, `ConnectAttach`, `ConnectDetach`).
//!
//! A channel holds the threads involved in message passing on it: RECEIVE-
//! blocked servers, SEND-blocked clients (a priority FIFO), REPLY-blocked
//! clients, and queued pulses. All of it is protected by the channel lock;
//! see `ker::msg` for how threads move between these.

use alloc::collections::VecDeque;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::KerResult;
use super::errno::*;
use super::process::{self, Process};
use crate::sched::{Thread, ThreadRef};
use crate::sync::IrqSpinLock;

/// `_NTO_CHF_FIXED_PRIORITY`: no priority inheritance on this channel.
pub const CHF_FIXED_PRIORITY: u32 = 0x0001;
/// `_NTO_CHF_PULSE_POOL`.
const CHF_PULSE_POOL: u32 = 0x0100;

/// A pulse waiting on a channel.
#[derive(Debug, Clone, Copy)]
pub struct Pulse {
    pub priority: u8,
    pub code: i8,
    pub value: u64,
    pub scoid: i32,
}

#[derive(Default)]
pub struct ChannelState {
    /// RECEIVE-blocked threads, in arrival order.
    pub receivers: VecDeque<ThreadRef>,
    /// RECEIVE-blocked threads that take only pulses (`MsgReceivePulsev()`).
    pub pulse_receivers: VecDeque<ThreadRef>,
    /// SEND-blocked threads, by priority then arrival.
    pub senders: Vec<ThreadRef>,
    /// REPLY-blocked threads (received, not yet replied to).
    pub repliers: Vec<ThreadRef>,
    /// Pending pulses, by priority then arrival.
    pub pulses: VecDeque<Pulse>,
    /// The thread that last received on this channel (server boost).
    pub last_receiver: Option<ThreadRef>,
    /// Destroyed (or its process has ended): sends fail with ESRCH.
    pub destroyed: bool,
    /// Its pulse pool is configured (`_NTO_CCTL_PULSE_POOL`).
    pub pulse_pool: bool,
}

pub struct Channel {
    pub chid: i32,
    pub owner: Owner,
    pub flags: u32,
    pub state: IrqSpinLock<ChannelState>,
}

/// A channel's owner (by pid: a channel doesn't keep its process
/// alive).
pub struct Owner {
    pub pid: i32,
}

/// A process's connection to a channel.
pub struct Connection {
    pub channel: Arc<Channel>,
    /// Server connection ID, as the server sees this connection.
    pub scoid: i32,
    /// `FD_CLOEXEC` (`ConnectFlags`).
    pub flags: AtomicU32,
    /// The kernel pipe it is bound to (`PipeOpen()`), if any.
    pub pipe: crate::sync::IrqSpinLock<Option<alloc::sync::Arc<super::kpipe::Binding>>>,
}

impl Connection {
    /// A new connection of process `client` to `channel`, with the
    /// server connection ID that client has there (`scoid_for`).
    pub fn attach(channel: Arc<Channel>, client: i32) -> Self {
        let scoid = scoid_for(&channel, client, false);
        Self::new(channel, scoid)
    }

    /// A copy of this connection for process `client` (fork, spawn): to
    /// the same channel, sharing or not as this one does.
    pub fn copy_for(&self, client: i32) -> Self {
        let own = SCOIDS
            .lock()
            .get(&(self.channel.owner.pid, self.scoid & !SCOID_BIT))
            .is_some_and(|e| e.own);
        let scoid = scoid_for(&self.channel, client, own);
        let copy = Self::new(self.channel.clone(), scoid);
        // A pipe's end goes with its copy.
        super::kpipe::copy_binding(self, &copy);
        copy
    }

    pub fn new(channel: Arc<Channel>, scoid: i32) -> Self {
        Self {
            channel,
            scoid,
            flags: AtomicU32::new(0),
            pipe: crate::sync::IrqSpinLock::new(None),
        }
    }
}

/// `_NTO_CHF_DISCONNECT`: the server wants `_PULSE_CODE_DISCONNECT`.
const CHF_DISCONNECT: u32 = 0x8;
/// `_PULSE_CODE_DISCONNECT`: a connection to the channel has gone (the
/// pulse carries its scoid).
const PULSE_CODE_DISCONNECT: i32 = -33;

impl Drop for Connection {
    /// The last descriptor referring to the connection is gone (closed,
    /// replaced by dup2(), or its process ended). If it was its client's
    /// last connection to the channel, tell the server, which then closes
    /// what it had open for that scoid and releases it (`ConnectDetach`).
    fn drop(&mut self) {
        super::kpipe::disconnect(self);
        let disconnect = self.channel.flags & CHF_DISCONNECT != 0;
        // At the priority of the thread whose close or end drops it, as
        // procnto sends it (measured: a client at 12 ending; review IPC-14).
        if scoid_release(&self.channel, self.scoid, disconnect) && disconnect {
            let prio = crate::sched::current().priority();
            super::msg::channel_pulse(&self.channel, prio, PULSE_CODE_DISCONNECT, 0, self.scoid);
        }
    }
}

/// `_NTO_COF_CLOEXEC`, the one connection flag so far (= `FD_CLOEXEC`).
pub const COF_CLOEXEC: u32 = 0x1;
/// `_NTO_COF_DUP2`: `ConnectAttach(0, 0, oldfd, newfd, _NTO_COF_DUP2)`.
const COF_DUP2: u32 = 0x8000;

/// `dup2()` in the kernel (QNX 8's `ker_connect_attach`): file descriptor
/// `to` becomes the same connection as `from`, replacing what it was.
/// Both must be file descriptors (not side channels); `to` below
/// `RLIMIT_NOFILE`, else EMFILE, and below `FD_INDEX_LIMIT`, else EBADF
/// (measured, IPC-1: 3000 with the limit at 3000 is EMFILE, 70000 and
/// -1 are EBADF).
fn dup2(from: i32, to: i32) -> KerResult {
    if from < 0 || to < 0 || (from | to) as u32 & process::SIDE_CHANNEL != 0 {
        return Err(EBADF);
    }
    let p = process::current();
    if to as u32 >= process::FD_INDEX_LIMIT {
        return Err(EBADF);
    }
    if to as u32 >= p.nofile() {
        return Err(EMFILE);
    }
    if from != to {
        p.dup_connection(from, to).ok_or(EBADF)?;
    }
    Ok(to as u64)
}

/// Server connection IDs have bit 16 set, which tells them from
/// connection IDs (QNX's `ker_connect_detach` relies on it).
const SCOID_BIT: i32 = 0x1_0000;

/// A server connection ID in use: a client process's connections to a
/// channel (or one connection made with `_NTO_COF_NOSHARE`, `own`).
/// `held`: they are all gone and the server was told
/// (`_PULSE_CODE_DISCONNECT`), but hasn't released it yet. `mq`: the
/// message queue they are bound to (`MsgQueueOpen()`): procnto binds the
/// server connection, so a client's connections to a queue manager's
/// channel share one binding (measured: a second `MsgQueueOpen()` for
/// another of them is EBUSY), but for `_NTO_COF_NOSHARE` ones.
struct Scoid {
    client: i32,
    chid: i32,
    conns: u32,
    held: bool,
    own: bool,
    mq: Option<super::mqueue::Binding>,
}

/// Server connection IDs, as procnto's (measured): one per client process
/// and channel, shared by all the client's connections there but those
/// made with `_NTO_COF_NOSHARE` (libc's to a pipe manager, which
/// `PipeOpen()` binds one by one: procnto's `pipe_get_connection`), numbered
/// per server process from 1 (the lowest free one), by (server pid,
/// number). It is released when the client's last connection goes, or,
/// on a channel with `_NTO_CHF_DISCONNECT`, when the server detaches it.
static SCOIDS: IrqSpinLock<alloc::collections::BTreeMap<(i32, i32), Scoid>> =
    IrqSpinLock::new(alloc::collections::BTreeMap::new());

/// The server connection ID of a new connection of process `client` to
/// `channel`: the one the client already has there, or a new one (always,
/// for a connection not to share, `own`).
pub fn scoid_for(channel: &Channel, client: i32, own: bool) -> i32 {
    let server = channel.owner.pid;
    let mut t = SCOIDS.lock();
    let mine = t
        .range_mut((server, 1)..(server, SCOID_BIT))
        .find(|(_, e)| !own && !e.own && e.client == client && e.chid == channel.chid && !e.held);
    if let Some((&(_, n), e)) = mine {
        e.conns += 1;
        return SCOID_BIT | n;
    }
    let mut n = 1;
    for (&(_, k), _) in t.range((server, 1)..(server, SCOID_BIT)) {
        if k != n {
            break;
        }
        n += 1;
    }
    t.insert((server, n), Scoid { client, chid: channel.chid, conns: 1, held: false, own, mq: None });
    SCOID_BIT | n
}

/// A connection with `scoid` to `channel` has gone; true if it was its
/// client's last there. The ID is then held for the server to release
/// (`disconnect`), or free.
fn scoid_release(channel: &Channel, scoid: i32, disconnect: bool) -> bool {
    let key = (channel.owner.pid, scoid & !SCOID_BIT);
    let mut t = SCOIDS.lock();
    let Some(e) = t.get_mut(&key) else {
        return false;
    };
    e.conns = e.conns.saturating_sub(1);
    if e.conns > 0 {
        return false;
    }
    // Its queue binding goes with its last connection (dropped unlocked).
    let _mq = e.mq.take();
    if disconnect {
        e.held = true;
    } else {
        t.remove(&key);
    }
    drop(t);
    true
}

/// Run `f` on the message queue binding of `conn`'s server connection.
pub fn with_queue_binding<R>(conn: &Connection, f: impl FnOnce(&mut Option<super::mqueue::Binding>) -> R) -> R {
    let mut t = SCOIDS.lock();
    let mut none = None;
    let slot = match t.get_mut(&(conn.channel.owner.pid, conn.scoid & !SCOID_BIT)) {
        Some(e) => &mut e.mq,
        None => &mut none,
    };
    f(slot)
}

/// The client process of the calling server's connection ID `scoid`.
pub fn scoid_client(server: i32, scoid: i32) -> Option<i32> {
    SCOIDS.lock().get(&(server, scoid & !SCOID_BIT)).map(|e| e.client)
}

/// A server process has ended: its connection IDs go with it.
pub fn server_exited(server: i32) {
    SCOIDS.lock().retain(|&(s, _), _| s != server);
}

impl ChannelState {
    /// Insert into a priority FIFO: after all entries of equal or higher priority.
    pub fn add_sender(&mut self, t: &Thread) {
        let at = self
            .senders
            .iter()
            .position(|s| s.priority() < t.priority())
            .unwrap_or(self.senders.len());
        self.senders.insert(at, t.arc());
    }

    pub fn add_pulse(&mut self, p: Pulse) {
        let at = self
            .pulses
            .iter()
            .position(|q| q.priority < p.priority)
            .unwrap_or(self.pulses.len());
        self.pulses.insert(at, p);
    }

    pub fn take_replier(&mut self, t: &Thread) -> bool {
        match self.repliers.iter().position(|r| core::ptr::eq(&**r, t)) {
            Some(i) => {
                self.repliers.swap_remove(i);
                true
            }
            None => false,
        }
    }
}

pub fn channel_create_in(owner: &Process, flags: u32) -> i32 {
    owner.add_channel(|chid| Channel {
        chid,
        owner: Owner { pid: owner.pid },
        flags,
        state: IrqSpinLock::new(ChannelState::default()),
    })
}

/// `ChannelCreate(flags)`: a public channel (one anyone may connect to,
/// without `_NTO_CHF_PRIVATE`) needs `PROCMGR_AID_PUBLIC_CHANNEL`.
pub fn channel_create(flags: u32) -> KerResult {
    const PRIVATE: u32 = 0x1000;
    // Flags procnto doesn't take (`_NTO_CHF_NET_MSG`, `_REPLY_LEN`,
    // `_GLOBAL`, `_UNBLOCK_TIMER` and unknown ones): EINVAL.
    const UNSUPPORTED: u32 = 0xffff_8e90;
    if flags & UNSUPPORTED != 0 {
        return Err(EINVAL);
    }
    if flags & PRIVATE == 0 {
        super::ability::check(super::ability::PUBLIC_CHANNEL, None)?;
    }
    Ok(channel_create_in(process::current(), flags) as u64)
}

/// `ChannelDestroy(chid)`: blocked threads are released with ESRCH.
pub fn channel_destroy(chid: i32) -> KerResult {
    let ch = process::current().remove_channel(chid).ok_or(ESRCH)?;
    destroy(&ch);
    Ok(0)
}

/// Empty channel `ch` (destroyed, or its process has ended): its blocked
/// threads are released with ESRCH, its pulses dropped.
pub fn destroy(ch: &Channel) {
    let mut st = ch.state.lock();
    let st = &mut *st;
    let mut threads: Vec<ThreadRef> = st.receivers.drain(..).collect();
    threads.extend(st.pulse_receivers.drain(..));
    threads.append(&mut st.senders);
    threads.append(&mut st.repliers);
    st.pulses.clear();
    st.last_receiver = None;
    st.destroyed = true;
    for t in threads {
        super::msg::fail_blocked(&t, ESRCH);
    }
}

// `ChannelCtl()` commands (`<sys/neutrino.h>`)
const CCTL_PULSE_POOL: u32 = 1;
const CCTL_WIDE: u32 = 2;
/// The most lanes of a wide channel.
const MAX_LANES: usize = 64;

/// `ChannelCtl(chid, cmd, data)`.
/// - `_NTO_CCTL_WIDE` (undocumented; io-sock's `qnx.chan_width`): `data`
///   points to a lane count n. The channel is then also named by the n - 1
///   channel IDs after `chid` (EBUSY if one is taken), its lanes; io-sock
///   receives on each with its own threads. In OsCraft the lanes share the
///   channel's queues.
/// - `_NTO_CCTL_PULSE_POOL` (`ChannelCreatePulsePool()`), `data` a
///   `struct nto_channel_config` { sigevent, num_pulses, rearm_threshold,
///   ... }, on a channel created with `_NTO_CHF_PULSE_POOL`: OsCraft
///   allocates pulses as they come, so the pool never runs dry and its
///   event never fires.
pub fn channel_ctl(chid: i32, cmd: u32, data: u64) -> KerResult {
    let ch = process::current().channel(chid).ok_or(ESRCH)?;
    match cmd {
        CCTL_WIDE => {
            let mut b = [0u8; 4];
            super::umem::read(super::umem::Space::current(), data, &mut b)?;
            let lanes = u32::from_le_bytes(b) as usize;
            if !(1..=MAX_LANES).contains(&lanes) || ch.chid != chid {
                return Err(EINVAL);
            }
            if !process::current().widen_channel(chid, lanes) {
                return Err(EBUSY);
            }
            Ok(0)
        }
        CCTL_PULSE_POOL => {
            if ch.flags & CHF_PULSE_POOL == 0 {
                return Err(EINVAL);
            }
            let mut b = [0u8; 4];
            let space = super::umem::Space::current();
            super::umem::read(space, data + super::event::SIGEV_SIZE as u64, &mut b)?;
            if u32::from_le_bytes(b) == 0 {
                return Err(EINVAL);
            }
            let mut st = ch.state.lock();
            if st.pulse_pool {
                return Err(EBUSY);
            }
            st.pulse_pool = true;
            Ok(0)
        }
        _ => Err(ENOSYS),
    }
}

/// `ConnectAttach(nd, pid, chid, index, flags)`.
pub fn connect_attach(nd: u32, pid: i32, chid: i32, index: u32, flags: u32) -> KerResult {
    if flags & COF_DUP2 != 0 {
        return dup2(chid, index as i32);
    }
    // The node descriptor is ignored (procnto's `ker_connect_attach`).
    let _ = nd;
    let server = process::lookup(pid).ok_or(ESRCH)?;
    let channel = server.channel(chid).ok_or(ESRCH)?;
    // A private channel takes connections from its own process only
    // (procnto: EPERM).
    const CHF_PRIVATE: u32 = 0x1000;
    if channel.flags & CHF_PRIVATE != 0 && server.pid != process::current().pid {
        return Err(EPERM);
    }
    // A file descriptor is chosen by the index: at or above
    // `FD_INDEX_LIMIT` it is EINVAL, at or above `RLIMIT_NOFILE` EMFILE,
    // as is a space with nothing free from it up (measured, IPC-1).
    // Checked before the server gets a connection ID for it: a refused
    // attach must not leave it a disconnect pulse for an ID it never saw.
    let me = process::current();
    if index & process::SIDE_CHANNEL == 0 {
        if index >= process::FD_INDEX_LIMIT {
            return Err(EINVAL);
        }
        if index >= me.nofile() {
            return Err(EMFILE);
        }
    }
    // `_NTO_COF_NOSHARE`: a server connection ID of its own.
    const COF_NOSHARE: u32 = 0x40;
    let scoid = scoid_for(&channel, me.pid, flags & COF_NOSHARE != 0);
    let conn = Connection::new(channel, scoid);
    conn.flags.store(flags & COF_CLOEXEC, Ordering::Relaxed);
    let coid = me.add_connection(index, conn).ok_or(EMFILE)?;
    Ok(coid as u64)
}

/// Bit of a `ConnectDetach` argument that makes it a server's scoid.
const DETACH_SCOID: i32 = SCOID_BIT;

/// `ConnectDetach(coid)`. A server's `ConnectDetach(scoid | 0x10000)`
/// (after `_PULSE_CODE_DISCONNECT`) releases the server connection ID.
pub fn connect_detach(coid: i32) -> KerResult {
    if coid & DETACH_SCOID != 0 && coid as u32 & process::SIDE_CHANNEL == 0 {
        let key = (process::current().pid, coid & !SCOID_BIT);
        let mut t = SCOIDS.lock();
        if t.get(&key).is_some_and(|e| e.held) {
            t.remove(&key);
        }
        return Ok(0);
    }
    let me = process::current();
    let conn = me.remove_connection(coid).ok_or(EINVAL)?;
    // A thread blocked in a send on it leaves (or the server is asked);
    // the connection itself goes with the last reference, which such a
    // thread holds until its send is over.
    super::msg::connection_detached(me, coid, &conn.channel);
    drop(conn);
    Ok(0)
}

/// `_client_info.flags` (`_NTO_CI_*`), those procnto sets: not in its
/// terminal's foreground group, stopped, chroot()ed, its loader at work
/// (servers accept `_IO_DUP` of another process's descriptors from the
/// loader), and an ability check failed.
const CI_BKGND_PGRP: u32 = 0x4;
const CI_STOPPED: u32 = 0x80;
const CI_UNABLE: u32 = 0x100;
const CI_CHROOT: u32 = 0x800;
const CI_LOADER: u32 = 0x8000;
/// In `ngroups`: check the abilities passed (libc's
/// `ConnectClientInfoAble()` sets it).
const NGROUPS_ABLE: u32 = 0x8000_0000;

/// `sizeof(struct _client_able)`: ability, flags, range_lo, range_hi.
const CLIENT_ABLE_SIZE: u64 = 24;

/// `ConnectClientInfoExtraArgs(scoid, info, ngroups, abilities, nable,
/// type_id)`: the process behind server connection `scoid` (-1: the
/// caller itself): its user and group IDs, with up to `ngroups` of its
/// supplementary groups; `sid` is its session; its security type ID is 0.
/// As procnto's `ker_connect_client_info`, with `NGROUPS_ABLE` in
/// `ngroups` each of the `nable` `struct _client_able`s gets `flags` 1 if
/// the client has that ability for `range_lo` to `range_hi`, else 0, and
/// any 0 sets `_NTO_CI_UNABLE` (EINVAL for abilities to check but no
/// array). An array with no entries is EFAULT.
pub fn connect_client_info(
    scoid: i32,
    info: u64,
    ngroups: i32,
    abilities: u64,
    nable: i32,
    type_id: u64,
) -> KerResult {
    let able = ngroups as u32 & NGROUPS_ABLE != 0;
    let ngroups = (ngroups as u32 & !NGROUPS_ABLE) as usize;
    let nable = nable as u32 as u64;
    if able && abilities == 0 && nable != 0 {
        return Err(EINVAL);
    }
    if abilities != 0 && nable == 0 {
        return Err(EFAULT);
    }
    // As procnto: -1 is the caller; a server connection ID must have
    // `_NTO_CONNECTION_SCOID`; anything else EINVAL.
    const CONNECTION_SCOID: i32 = 0x1_0000;
    let p = match scoid {
        -1 => process::current().arc(),
        s if s & CONNECTION_SCOID == 0 => return Err(EINVAL),
        s => process::by_scoid(process::current().pid, s).ok_or(EINVAL)?,
    };
    // `struct _client_info` (`<sys/neutrino.h>`): nd, pid, sid, flags,
    // then `struct _cred_info`.
    let mut b = alloc::vec![0u8; 16];
    b[4..8].copy_from_slice(&p.pid.to_le_bytes());
    b[8..12].copy_from_slice(&p.sid().to_le_bytes());
    let mut flags = 0;
    if !p.root.lock().is_empty() {
        flags |= CI_CHROOT;
    }
    if crate::proc::session::foreground(p.sid()).is_some_and(|fg| fg != p.pgid()) {
        flags |= CI_BKGND_PGRP;
    }
    if p.debug.lock().job_stopped {
        flags |= CI_STOPPED;
    }
    if p.loading.load(Ordering::Relaxed) {
        flags |= CI_LOADER;
    }
    let space = super::umem::Space::current;
    if able && abilities != 0 {
        for i in 0..nable {
            let at = abilities + i * CLIENT_ABLE_SIZE;
            let mut e = [0u8; CLIENT_ABLE_SIZE as usize];
            super::umem::read(space(), at, &mut e)?;
            let aid = u32::from_le_bytes(e[0..4].try_into().unwrap());
            let lo = u64::from_le_bytes(e[8..16].try_into().unwrap());
            let hi = u64::from_le_bytes(e[16..24].try_into().unwrap());
            let ok = super::ability::allowed_range(&p, aid, lo, hi);
            if !ok {
                flags |= CI_UNABLE;
            }
            super::umem::write(space(), at + 4, &(ok as u32).to_le_bytes())?;
        }
    }
    if info != 0 {
        b[12..16].copy_from_slice(&flags.to_le_bytes());
        b.extend(p.cred.lock().info(ngroups));
        super::umem::write(space(), info, &b)?;
    }
    if type_id != 0 {
        super::umem::write(space(), type_id, &0u32.to_le_bytes())?;
    }
    Ok(0)
}

/// `sizeof(struct _server_info)` (`struct _msg_info64`).
const SERVER_INFO_SIZE: usize = 64;

/// `ConnectServerInfo(pid, coid, info)`: the connection `coid` of process
/// `pid` (0: the caller), or the next one above it in the same space.
/// Returns its ID and, if `info` is set, fills in the server's nd, pid,
/// chid and scoid and the connection ID.
pub fn connect_server_info(pid: i32, coid: i32, info: u64) -> KerResult {
    let p = process::lookup(pid).ok_or(ESRCH)?;
    let (coid, conn) = p.next_connection(coid).ok_or(EINVAL)?;
    if info != 0 {
        let mut b = [0u8; SERVER_INFO_SIZE];
        b[8..12].copy_from_slice(&conn.channel.owner.pid.to_le_bytes());
        b[16..20].copy_from_slice(&conn.channel.chid.to_le_bytes());
        b[20..24].copy_from_slice(&conn.scoid.to_le_bytes());
        b[24..28].copy_from_slice(&coid.to_le_bytes());
        // The connection's flags (`_NTO_COF_*`): close-on-exec, bound to
        // a kernel pipe.
        let mut flags = conn.flags.load(Ordering::Relaxed) & COF_CLOEXEC;
        if conn.pipe.lock().is_some() {
            flags |= super::kpipe::COF_PIPE;
        }
        b[30..32].copy_from_slice(&(flags as u16).to_le_bytes());
        // OsCraft's, in `reserved`, to a FreeBSD process: 1 if the server
        // is a QNX binary, whose messages take QNX's layouts. A QNX
        // caller's stays procnto's, 0.
        if process::current().personality() == process::Personality::Native
            && process::lookup(conn.channel.owner.pid).is_some_and(|s| s.personality() == process::Personality::Qnx)
        {
            b[60..64].copy_from_slice(&1u32.to_le_bytes());
        }
        super::umem::write(super::umem::Space::current(), info, &b)?;
    }
    Ok(coid as u64)
}

/// `ConnectFlags(pid, coid, mask, bits)`: set the flags in `mask` to
/// `bits`; returns the old flags.
pub fn connect_flags(pid: i32, coid: i32, mask: u32, bits: u32) -> KerResult {
    let p = process::lookup(pid).ok_or(ESRCH)?;
    let conn = p.connection(coid).ok_or(EBADF)?;
    let mask = mask & COF_CLOEXEC;
    let old = conn
        .flags
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |f| {
            Some((f & !mask) | (bits & mask))
        })
        .unwrap();
    Ok(old as u64)
}

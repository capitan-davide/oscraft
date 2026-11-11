//! Events (`struct sigevent`, `<sys/siginfo.h>`): how the kernel notifies
//! a process of an interrupt, a timer's expiry or a server's
//! `MsgDeliverEvent()`.
//!
//! Layout (AArch64): `sigev_notify` at 0 (the type in the low byte, flags
//! above), `sigev_signo`/`sigev_coid` at 8, `sigev_value` at 16,
//! `sigev_code` (16 bits) at 24 and `sigev_priority` (16 bits) at 26.
//!
//! Delivered so far: `SIGEV_NONE`, the signal types (to the process; a
//! thread's own signals are not yet separate), `SIGEV_PULSE` (on the
//! process's connection `sigev_coid`, or with a negative one on its
//! channel -`sigev_coid`; priority -1 is the target thread's),
//! `SIGEV_INTR` (for `InterruptWait()`), `SIGEV_THREAD` (a new thread
//! runs `sigev_notify_function(sigev_value)`, see `thread::sigev_thread`),
//! `SIGEV_MEMORY` (an atomic operation `sigev_memop` with
//! `sigev_value.sival_int` on the word at `sigev_addr`, 64 bits at 8; from
//! an interrupt, done by a kernel thread; quietly nothing if the word
//! isn't writable; an unknown operation is EINVAL when delivered, so a
//! timer's is dropped, as procnto) and `SIGEV_SEM` (a post of the named
//! semaphore open on connection `sigev_coid`, see `sync::named_post`).
//!
//! Registered events (QNX `ker_msg_register_event`,
//! `sigevent_lookup_handle`, `ker_msg_deliver_event`):
//! - `MsgRegisterEvent(event, coid, &handle)` keeps a copy of the event in
//!   the caller's table and stores its index, the handle. `coid` names the
//!   server allowed to deliver it (its channel's owner;
//!   `_NTO_REGEVENT_ALLOW_ANY_SERVER` -1: any; `_NTO_REGEVENT_ALLOW_SELF`
//!   -2: the caller). EINVAL for an event that is a handle already, or a
//!   connection that isn't one. A null event unregisters `handle` (ESRCH if
//!   unknown).
//! - An event with `SIGEV_FLAG_HANDLE` carries the handle in `sigev_signo`.
//!   Delivering it delivers the registered event, if the deliverer may;
//!   its value and code replace the registered ones only if the event was
//!   registered `SIGEV_FLAG_UPDATEABLE` / `SIGEV_FLAG_CODE_UPDATEABLE`.
//! - `MsgDeliverEvent(rcvid, event)`: a server delivers `event` to the
//!   client thread `rcvid` (even after replying to it); `rcvid` 0 is the
//!   calling thread. A server may deliver only registered events (a
//!   handle): any other is EACCES, unless `procnto -U` (QNX 8: "By
//!   default, you can use only registered events", the sigevent page;
//!   checked against procnto, all kinds of events). Delivering to itself
//!   (`rcvid` 0) needs none.
//! - `MsgVerifyEvent(rcvid, event)`: the same checks, nothing delivered;
//!   a pulse on a connection that doesn't exist is EBADF.
//! - The kernel's own deliveries (timers, interrupts) count as procnto's.

use core::sync::atomic::{AtomicBool, Ordering};

use super::KerResult;
use super::errno::{EACCES, EAGAIN, EBADF, EINVAL, ENOSYS, EPERM, ESRCH};
use super::process::{self, Process};
use super::signal::{self, SigInfo};
use super::umem::{self, Space};
use crate::sched::{self, ThreadRef};
use crate::sync::IrqSpinLock;

/// Orders a `SIGEV_INTR` delivery with the target's `InterruptWait`.
pub static INTR_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

pub const SIGEV_SIZE: usize = 32;
const TYPE_MASK: u32 = 0xff;
pub const FLAG_HANDLE: u32 = 0x2000;
pub const FLAG_UPDATEABLE: u32 = 0x400;
const FLAG_CODE_UPDATEABLE: u32 = 0x4000;
/// `_NTO_REGEVENT_ALLOW_ANY_SERVER`, `_NTO_REGEVENT_ALLOW_SELF`
const ALLOW_ANY: i32 = -1;
const ALLOW_SELF: i32 = -2;
/// `SI_USER`: the `si_code` of a signal a server delivers.
const SI_USER: i32 = 0x10001;

pub const SIGEV_NONE: u32 = 0;
pub const SIGEV_SIGNAL: u32 = 1;
pub const SIGEV_SIGNAL_CODE: u32 = 2;
pub const SIGEV_SIGNAL_THREAD: u32 = 3;
pub const SIGEV_PULSE: u32 = 4;
pub const SIGEV_UNBLOCK: u32 = 5;
pub const SIGEV_INTR: u32 = 6;
pub const SIGEV_THREAD: u32 = 7;
pub const SIGEV_MEMORY: u32 = 8;
pub const SIGEV_SEM: u32 = 9;
// `SIGEV_MEM_ASSIGN` ... `SIGEV_MEM_BITTOGGLE`
const MEM_ASSIGN: u32 = 1;
const MEM_ADD: u32 = 2;
const MEM_SUB: u32 = 3;
const MEM_BITSET: u32 = 4;
const MEM_BITCLR: u32 = 5;
const MEM_BITTOGGLE: u32 = 6;

#[derive(Debug, Clone, Copy, Default)]
pub struct SigEvent {
    pub notify: u32,
    /// `sigev_signo` or `sigev_coid`.
    pub un1: i32,
    pub value: u64,
    pub code: i16,
    pub priority: i16,
    /// `sigev_notify_function` (`SIGEV_THREAD`; 64 bits at 8).
    pub function: u64,
    /// `sigev_notify_attributes` (`SIGEV_THREAD`; 64 bits at 24).
    pub attributes: u64,
}

impl SigEvent {
    pub fn from_bytes(b: &[u8; SIGEV_SIZE]) -> Self {
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        SigEvent {
            notify: u32_at(0),
            un1: u32_at(8) as i32,
            value: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            code: i16::from_le_bytes([b[24], b[25]]),
            priority: i16::from_le_bytes([b[26], b[27]]),
            function: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            attributes: u64::from_le_bytes(b[24..32].try_into().unwrap()),
        }
    }

    pub fn kind(&self) -> u32 {
        self.notify & TYPE_MASK
    }

    /// Check that the event can be delivered (when it is given to the
    /// kernel, so errors show then rather than when it fires).
    pub fn validate(&self) -> Result<(), u32> {
        if self.notify & FLAG_HANDLE != 0 {
            // Checked when registered.
            return Ok(());
        }
        match self.kind() {
            SIGEV_NONE | SIGEV_PULSE | SIGEV_UNBLOCK | SIGEV_INTR | SIGEV_MEMORY | SIGEV_SEM => Ok(()),
            SIGEV_THREAD if self.function != 0 => Ok(()),
            SIGEV_THREAD => Err(EINVAL),
            SIGEV_SIGNAL | SIGEV_SIGNAL_CODE | SIGEV_SIGNAL_THREAD
                if (1..signal::NSIG as i32).contains(&self.un1) =>
            {
                Ok(())
            }
            SIGEV_SIGNAL | SIGEV_SIGNAL_CODE | SIGEV_SIGNAL_THREAD => Err(EINVAL),
            _ => Err(ENOSYS),
        }
    }
}

/// Read the `struct sigevent` at `addr` in `space`.
pub fn read(space: Space, addr: u64) -> Result<SigEvent, u32> {
    let mut b = [0u8; SIGEV_SIZE];
    umem::read(space, addr, &mut b)?;
    let ev = SigEvent::from_bytes(&b);
    ev.validate()?;
    // SIGEV_THREAD takes PROCMGR_AID_SIGEV_THREAD to register.
    if ev.kind() == SIGEV_THREAD {
        super::ability::check(super::ability::SIGEV_THREAD, None)?;
    }
    Ok(ev)
}

/// Deliver `ev` to process `p`, on behalf of its thread `t` (the one that
/// registered the event). `si_code` is the code a signal gets, unless the
/// event names its own (`SIGEV_SIGNAL_CODE`, `SIGEV_SIGNAL_THREAD`).
/// Callable from interrupt handlers.
pub fn deliver(p: &Process, t: Option<ThreadRef>, ev: &SigEvent, si_code: i32) -> Result<(), u32> {
    deliver_from(p, t, ev, si_code, 0)
}

/// `deliver`, for timer `timer` (`ker::timer`'s key): a signal carries it
/// (`SigInfo::timer`).
pub fn deliver_from(p: &Process, t: Option<ThreadRef>, ev: &SigEvent, si_code: i32, timer: u64) -> Result<(), u32> {
    let resolved;
    let ev = if ev.notify & FLAG_HANDLE != 0 {
        resolved = resolve(p, super::process::KERNEL_PID, ev)?;
        &resolved
    } else {
        ev
    };
    match ev.kind() {
        SIGEV_NONE | SIGEV_UNBLOCK => Ok(()),
        kind @ (SIGEV_SIGNAL | SIGEV_SIGNAL_CODE | SIGEV_SIGNAL_THREAD) => {
            let code = match kind {
                SIGEV_SIGNAL => si_code,
                _ => crate::personality::qnx::code_in(ev.code as i32),
            };
            let info = SigInfo {
                code,
                pid: p.pid,
                value: ev.value,
                timer,
                ..Default::default()
            };
            // To the thread the event is for (the one that registered it,
            // made the timer...). One that has ended doesn't pass it on to
            // a new thread with its ID: the process's thread 1 takes it
            // (procnto, measured for a timer), or the process if that has
            // ended too.
            let t = match (kind, t) {
                (SIGEV_SIGNAL_THREAD, Some(t)) if t.state() == sched::State::Dead => {
                    sched::thread_of(p, 1)
                }
                (SIGEV_SIGNAL_THREAD, t) => t,
                _ => None,
            };
            match t {
                Some(t) => signal::post_thread(p, &t, p.personality().signo_in(ev.un1 as usize), info),
                None => signal::post(p, p.personality().signo_in(ev.un1 as usize), info),
            }
            Ok(())
        }
        SIGEV_PULSE => {
            // A negative `sigev_coid`: channel -coid of the receiver's.
            let (channel, scoid) = if ev.un1 < 0 {
                (p.channel(-ev.un1).ok_or(EBADF)?, 0)
            } else {
                let conn = p.connection(ev.un1).ok_or(EBADF)?;
                (conn.channel.clone(), conn.scoid)
            };
            let priority = match ev.priority {
                -1 => t.map_or(10, |t| t.priority()),
                n => n.clamp(1, 255) as u8,
            };
            super::msg::channel_pulse(&channel, priority, ev.code as i32, ev.value, scoid);
            Ok(())
        }
        // A new thread runs the function, created by a kernel thread (this
        // may be an interrupt handler): see `thread::sigev_thread`.
        SIGEV_THREAD => {
            let prio = t.as_ref().map_or(10, |t| t.priority());
            super::thread::sigev_thread(p.arc(), ev.function, ev.value, ev.attributes, prio);
            Ok(())
        }
        SIGEV_MEMORY => {
            let (addr, op, value) = (ev.function, ev.attributes as u32, ev.value as u32);
            if !(MEM_ASSIGN..=MEM_BITTOGGLE).contains(&op) {
                return Err(EINVAL);
            }
            if crate::percpu::this().in_irq.load(Ordering::Relaxed) {
                super::thread::sigev_memory(p.arc(), addr, op, value);
            } else {
                memory(p, addr, op, value);
            }
            Ok(())
        }
        SIGEV_SEM => {
            let client = crate::proc::resmgr::Client { pid: p.pid, coid: ev.un1 };
            let sem = crate::proc::resmgr::named_sem(client).ok_or(EBADF)?;
            super::sync::named_post(&sem).map(|_| ())
        }
        SIGEV_INTR => {
            let t = t.ok_or(EINVAL)?;
            let _g = INTR_LOCK.lock();
            t.intr_pending.store(true, Ordering::Release);
            if t.state() == sched::State::Intr {
                sched::wake(&t);
            }
            Ok(())
        }
        _ => Err(ENOSYS),
    }
}

/// A `SIGEV_MEMORY` operation `op` with `value` on the word at `addr` of
/// `p`; nothing if the word isn't there or writable.
pub fn memory(p: &Process, addr: u64, op: u32, value: u32) {
    // SAFETY: the caller holds `p`.
    let space = unsafe { Space::user(p) };
    let _ = umem::with_atomic_u32(space, addr, |a| {
        let _ = match op {
            MEM_ASSIGN => a.swap(value, Ordering::AcqRel),
            MEM_ADD => a.fetch_add(value, Ordering::AcqRel),
            MEM_SUB => a.fetch_sub(value, Ordering::AcqRel),
            MEM_BITSET => a.fetch_or(value, Ordering::AcqRel),
            MEM_BITCLR => a.fetch_and(!value, Ordering::AcqRel),
            _ => a.fetch_xor(value, Ordering::AcqRel),
        };
    });
}

// ANCHOR: resolve
/// The event `ev` stands for when process `deliverer` delivers it to `p`:
/// itself, or the one registered under its handle.
pub fn resolve(p: &Process, deliverer: i32, ev: &SigEvent) -> Result<SigEvent, u32> {
    if ev.notify & FLAG_HANDLE == 0 {
        return Ok(*ev);
    }
    let events = p.events.lock();
    let (mut reg, allowed) = usize::try_from(ev.un1)
        .ok()
        .and_then(|i| events.get(i).copied().flatten())
        .ok_or(EINVAL)?;
    if allowed != deliverer && allowed != ALLOW_ANY {
        return Err(EPERM);
    }
    if reg.notify & FLAG_UPDATEABLE != 0 {
        reg.value = ev.value;
    }
    if reg.notify & FLAG_CODE_UPDATEABLE != 0 {
        reg.code = ev.code;
    }
    Ok(reg)
}
// ANCHOR_END: resolve

/// `MsgRegisterEvent(event, coid, handlep)`.
pub fn msg_register_event(event: u64, coid: i32, handlep: u64) -> KerResult {
    let me = process::current();
    let space = Space::current();
    let mut h = [0u8; 4];
    if event == 0 {
        umem::read(space, handlep, &mut h)?;
        let i = u32::from_le_bytes(h) as usize;
        let mut events = me.events.lock();
        return match events.get_mut(i) {
            Some(slot @ Some(_)) => {
                *slot = None;
                Ok(0)
            }
            _ => Err(ESRCH),
        };
    }
    let mut b = [0u8; SIGEV_SIZE];
    umem::read(space, event, &mut b)?;
    let ev = SigEvent::from_bytes(&b);
    if ev.notify & FLAG_HANDLE != 0 {
        return Err(EINVAL);
    }
    ev.validate()?;
    let allowed = match coid {
        ALLOW_ANY => ALLOW_ANY,
        ALLOW_SELF => me.pid,
        c if c >= 0 => me.connection(c).ok_or(EINVAL)?.channel.owner.pid,
        _ => return Err(EINVAL),
    };
    let handle = {
        let mut events = me.events.lock();
        // At most `MAX_REGISTERED` per process, over all its connections
        // and channels: EAGAIN beyond, until one is unregistered (procnto,
        // measured).
        if events.iter().filter(|s| s.is_some()).count() >= MAX_REGISTERED {
            return Err(EAGAIN);
        }
        match events.iter().position(|s| s.is_none()) {
            Some(i) => {
                events[i] = Some((ev, allowed));
                i
            }
            None => {
                events.push(Some((ev, allowed)));
                events.len() - 1
            }
        }
    };
    umem::write(space, handlep, &(handle as u32).to_le_bytes())?;
    Ok(0)
}

/// The events a process may have registered at once (procnto, measured).
const MAX_REGISTERED: usize = 256;

/// Whether servers may deliver events their clients didn't register
/// (`procnto -U`); QNX 8 allows only registered ones by default.
static UNREGISTERED: AtomicBool = AtomicBool::new(false);

pub fn set_unregistered_events(on: bool) {
    UNREGISTERED.store(on, Ordering::Relaxed);
}

// ANCHOR: deliver_event
/// A server (`rcvid` not 0) may deliver only an event registered with
/// `MsgRegisterEvent()` (a handle), unless `procnto -U`: EACCES, for any
/// kind of event (QNX 8.0, checked against procnto). A thread delivering
/// to itself (`rcvid` 0) needs no registration.
fn check_registered(rcvid: i64, ev: &SigEvent) -> Result<(), u32> {
    if rcvid != 0 && ev.notify & FLAG_HANDLE == 0 && !UNREGISTERED.load(Ordering::Relaxed) {
        return Err(EACCES);
    }
    Ok(())
}

/// The thread an event goes to: the client behind `rcvid`, or the caller
/// for 0.
fn rcvid_thread(rcvid: i64) -> Result<ThreadRef, u32> {
    if rcvid == 0 {
        return Ok(sched::current().arc());
    }
    let t = u32::try_from(rcvid).ok().and_then(sched::thread).ok_or(ESRCH)?;
    // A message `qnxd` forwarded: the client it is for.
    Ok(t.ipc.client().unwrap_or(t))
}

/// `MsgDeliverEvent(rcvid, event)`.
pub fn msg_deliver_event(rcvid: i64, event: u64) -> KerResult {
    let client = rcvid_thread(rcvid)?;
    let mut b = [0u8; SIGEV_SIZE];
    umem::read(Space::current(), event, &mut b)?;
    let ev = SigEvent::from_bytes(&b);
    check_registered(rcvid, &ev)?;
    let target = client.process.clone();
    let ev = resolve(&target, process::current().pid, &ev)?;
    ev.validate()?;
    deliver(&target, Some(client), &ev, SI_USER)?;
    Ok(0)
}
// ANCHOR_END: deliver_event

/// `MsgVerifyEvent(rcvid, event)`: whether `event` could be delivered, by
/// the caller to itself (`rcvid` 0, e.g. a client before it gives the
/// event to a server) or by this server to the client behind `rcvid`: the
/// checks of `MsgDeliverEvent()`, nothing delivered.
pub fn msg_verify_event(rcvid: i64, event: u64) -> KerResult {
    let client = rcvid_thread(rcvid)?;
    let mut b = [0u8; SIGEV_SIZE];
    umem::read(Space::current(), event, &mut b)?;
    let ev = SigEvent::from_bytes(&b);
    check_registered(rcvid, &ev)?;
    let target = client.process.clone();
    let ev = resolve(&target, process::current().pid, &ev)?;
    ev.validate()?;
    if ev.kind() == SIGEV_PULSE
        && if ev.un1 < 0 { target.channel(-ev.un1).is_none() } else { target.connection(ev.un1).is_none() }
    {
        return Err(EBADF);
    }
    Ok(0)
}

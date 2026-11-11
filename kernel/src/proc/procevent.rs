//! Process manager events (`procmgr_event_notify()`,
//! `procmgr_event_notify_add()`, `procmgr_event_trigger()`;
//! `_PROC_EVENT`, `<sys/procmsg.h>`, `<sys/procmgr.h>`).
//!
//! The request is `struct _proc_event` { type, subtype, flags, sigevent }:
//! - `_PROC_EVENT_NOTIFY` (0): the sender's one notification, replaced
//!   (flags 0: removed).
//! - `_PROC_EVENT_NOTIFY_ADD` (2): one more notification; the reply's
//!   status is its id. `_PROC_EVENT_NOTIFY_DEL` (3) { type, subtype, id }
//!   removes it (ESRCH if it isn't the sender's).
//! - `_PROC_EVENT_TRIGGER` (1, and 4, `_UPDATEABLE`): deliver the events
//!   in `flags`; each `PROCMGR_EVENT_PRIVILEGED` bit needs
//!   `PROCMGR_AID_EVENT` (EPERM). A plain trigger's value is 0.
//!
//! The process manager delivers `PROCMGR_EVENT_PROCESS_CREATE`,
//! `PROCESS_DEATH`, `DAEMON_DEATH` (a process of session 1, where
//! `procmgr_daemon()` puts daemons, ended), `PATHSPACE` (a name was
//! attached or detached; the value is `pathspace_hash()` of its path),
//! `TOD` (`ClockTime()` set the time), `SYSCONF`/`CONFSTR` (a conf value
//! was set; the value is its name) and `CONTIG_ALLOC_FAIL` (contiguous
//! memory wasn't there). Events are delivered as procnto's, so a
//! registered one must allow procnto (`SYSMGR_COID`), and one that isn't
//! registered is never delivered, even with `procnto -U` (the
//! notification is accepted all the same; measured). A process's notifications go when
//! it ends.
//!
//! Watched values (`procmgr_value_notify_add()`, `procmgr_value_current()`;
//! `_PROC_VALUE` { type, subtype, vtype, value, id, sigevent }), as
//! procnto's `procmgr_value` and `procmgr_value_update`:
//! - `_PROC_VALUE` subtype 2 adds one, with an ID from the sender's event
//!   IDs (it goes with `_PROC_EVENT_NOTIFY_DEL`): `PROCMGR_VALUE_FREE_MEM`
//!   (the system's free memory, bytes) or `PROCMGR_VALUE_PRIVATE_MEM` of
//!   process `id` (its private memory, bytes; ESRCH for none, 0 included),
//!   with `PROCMGR_VALUE_TRIGGER_UP` and/or `_DOWN` (other bits EINVAL).
//!   A trigger delivers the event when the value crosses `value`: up, from
//!   below to it or above; down, from above to it or below; both, either.
//!   At once if it is there already.
//! - Subtype 0 answers its value now (ESRCH for no such ID), subtype 3
//!   deletes it.
//!
//! Values are checked after each memory manager request and when a
//! process ends (procnto checks as memory is reserved and released).

use alloc::vec::Vec;

use crate::arch::kercall::call;
use crate::ker::ability;
use crate::ker::errno::{EINVAL, ENOSYS, EPERM, ESRCH};
use crate::ker::event::{self, SIGEV_SIZE, SigEvent};
use crate::ker::number::*;
use crate::ker::process;
use crate::sync::IrqSpinLock;

pub const PROC_EVENT: u16 = 0x0019;
const NOTIFY: u16 = 0;
const TRIGGER: u16 = 1;
const NOTIFY_ADD: u16 = 2;
const NOTIFY_DEL: u16 = 3;
const TRIGGER_UPDATEABLE: u16 = 4;
const MSG_SIZE: usize = 8 + SIGEV_SIZE;

// PROCMGR_EVENT_* (<sys/procmgr.h>)
pub const PATHSPACE: u32 = 0x2;
pub const TOD: u32 = 0x4;
pub const SYSCONF: u32 = 0x1_0000;
pub const CONFSTR: u32 = 0x2_0000;
pub const DAEMON_DEATH: u32 = 0x4_0000;
pub const CONTIG_ALLOC_FAIL: u32 = 0x8_0000;
pub const PROCESS_DEATH: u32 = 0x10_0000;
pub const PROCESS_CREATE: u32 = 0x40_0000;
const PRIVILEGED: u32 = 0xffff_0000;

struct Notify {
    /// 0: the process's `_PROC_EVENT_NOTIFY` one.
    id: i32,
    pid: i32,
    flags: u32,
    event: SigEvent,
    /// A watched value (`_PROC_VALUE`), or none for process manager
    /// events.
    watch: Option<Watch>,
}

/// A watched value: the type with its trigger bits, the process for
/// `PRIVATE_MEM`, the trigger point and the value last seen.
#[derive(Clone, Copy)]
struct Watch {
    vtype: u32,
    pid: i32,
    point: u64,
    last: u64,
}

// PROCMGR_VALUE_* (<sys/procmgr.h>)
pub const PROC_VALUE: u16 = 0x001d;
const VALUE_PRIVATE_MEM: u32 = 0x1;
const VALUE_TRIGGER_UP: u32 = 0x4000_0000;
const VALUE_TRIGGER_DOWN: u32 = 0x8000_0000;
const VALUE_CURRENT: u16 = 0;
const VALUE_ADD: u16 = 2;
const VALUE_DEL: u16 = 3;

/// Whether any value is watched (so memory requests needn't look).
static WATCHING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

static NOTIFIES: IrqSpinLock<Vec<Notify>> = IrqSpinLock::new(Vec::new());
/// The next `_PROC_EVENT_NOTIFY_ADD` ID of each process that made one:
/// procnto numbers them per process, from 1.
static NEXT_ID: IrqSpinLock<alloc::collections::BTreeMap<i32, i32>> =
    IrqSpinLock::new(alloc::collections::BTreeMap::new());

/// The sender's next event ID: procnto numbers them per process, from 1.
fn next_id(pid: i32) -> i32 {
    let mut next = NEXT_ID.lock();
    let n = next.entry(pid).or_insert(1);
    *n += 1;
    *n - 1
}

/// Remove `pid`'s event `id` (ESRCH if it has none).
fn delete(pid: i32, id: i32) -> Result<(), u32> {
    let mut n = NOTIFIES.lock();
    let before = n.len();
    n.retain(|n| !(n.pid == pid && n.id == id));
    if n.len() == before {
        return Err(ESRCH);
    }
    Ok(())
}

/// The value `vtype` watches now (for `PRIVATE_MEM`, of process `pid`).
fn value_of(vtype: u32, pid: i32) -> Option<u64> {
    if vtype & VALUE_PRIVATE_MEM != 0 {
        let p = process::lookup(pid).filter(|_| pid != 0)?;
        let bytes = p.mem.lock().private_bytes();
        Some(bytes)
    } else {
        Some(crate::vm::pmem::free_pages() as u64 * crate::vm::pmem::PAGE_SIZE)
    }
}

/// Whether a value going from `old` to `new` crosses `w`'s trigger point
/// in a direction it watches.
fn crosses(w: &Watch, old: u64, new: u64) -> bool {
    (w.vtype & VALUE_TRIGGER_UP != 0 && old < w.point && w.point <= new)
        || (w.vtype & VALUE_TRIGGER_DOWN != 0 && new <= w.point && w.point < old)
}

/// Memory may have changed: deliver the watched values' events whose
/// trigger point was crossed since the last look.
pub fn values_changed() {
    if !WATCHING.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let watches: Vec<(i32, i32, Watch, SigEvent)> = NOTIFIES
        .lock()
        .iter()
        .filter_map(|n| n.watch.map(|w| (n.pid, n.id, w, n.event)))
        .collect();
    for (pid, id, w, ev) in watches {
        let Some(now) = value_of(w.vtype, w.pid) else { continue };
        if now == w.last {
            continue;
        }
        if let Some(n) = NOTIFIES.lock().iter_mut().find(|n| n.pid == pid && n.id == id)
            && let Some(nw) = n.watch.as_mut()
        {
            nw.last = now;
        }
        if crosses(&w, w.last, now) {
            deliver_to(pid, &ev);
        }
    }
}

/// Deliver `ev` to process `pid`, if it is registered (as `trigger`).
fn deliver_to(pid: i32, ev: &SigEvent) {
    if ev.notify & event::FLAG_HANDLE == 0 {
        return;
    }
    if let Some(p) = process::lookup(pid).filter(|_| pid != 0) {
        let _ = event::deliver(&p, None, ev, 0);
    }
}

/// Handle a `_PROC_VALUE` message from `sender`.
pub fn value(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    let word = |o: usize| msg.get(o..o + 4).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
    let id = word(16) as i32;
    match subtype {
        VALUE_ADD => {
            let vtype = word(4);
            if vtype & !(VALUE_PRIVATE_MEM | VALUE_TRIGGER_UP | VALUE_TRIGGER_DOWN) != 0 {
                return Err(EINVAL);
            }
            let point = msg.get(8..16).map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()));
            let now = value_of(vtype, id).ok_or(ESRCH)?;
            let b: &[u8; SIGEV_SIZE] = msg.get(24..24 + SIGEV_SIZE).ok_or(EINVAL)?.try_into().unwrap();
            let event = SigEvent::from_bytes(b);
            event.validate()?;
            let watch = Watch { vtype, pid: id, point, last: now };
            let handle = next_id(sender);
            NOTIFIES.lock().push(Notify {
                id: handle,
                pid: sender,
                flags: 0,
                event,
                watch: Some(watch),
            });
            WATCHING.store(true, core::sync::atomic::Ordering::Relaxed);
            reply(rcvid, handle as u64);
            // At once if the value is at the trigger point already.
            // UP is looked at first; DOWN only without it (procnto).
            let there = if vtype & VALUE_TRIGGER_UP != 0 {
                now >= point
            } else {
                vtype & VALUE_TRIGGER_DOWN != 0 && now <= point
            };
            if there {
                deliver_to(sender, &event);
            }
        }
        VALUE_CURRENT => {
            let w = NOTIFIES
                .lock()
                .iter()
                .find(|n| n.pid == sender && n.id == id)
                .and_then(|n| n.watch)
                .ok_or(ESRCH)?;
            let now = value_of(w.vtype, w.pid).unwrap_or(u64::MAX);
            let _ = call(__KER_MSG_REPLYV, [rcvid, 0, &now as *const u64 as u64, (-8i64) as u64, 0, 0]);
        }
        VALUE_DEL => {
            delete(sender, id)?;
            reply(rcvid, 0);
        }
        _ => return Err(ENOSYS),
    }
    Ok(())
}

/// Deliver the notifications that want one of `flags`. An updateable
/// event (`SIGEV_FLAG_UPDATEABLE`) carries `value` in `sival_int`, as
/// procnto's `procmgr_trigger()`: the pid created or ended, the path's
/// hash, the conf name, or what `procmgr_event_trigger_updateable()` gave.
/// Events raised where a lock the delivery may need is held
/// (`trigger_later`), delivered once it is dropped (`deliver_later`).
static LATER: IrqSpinLock<Vec<(u32, u32)>> = IrqSpinLock::new(Vec::new());

/// `trigger`, but later: for a path that holds an open-file context's
/// lock, which a `SIGEV_SEM` delivery takes to find the semaphore
/// (`resmgr::named_sem`; review PATHMEM-2). The caller's caller, back
/// outside the lock, calls `deliver_later`.
pub fn trigger_later(flags: u32, value: u32) {
    LATER.lock().push((flags, value));
}

/// Deliver what `trigger_later` queued, in order.
pub fn deliver_later() {
    loop {
        let next = LATER.lock().first().copied();
        match next {
            Some((flags, value)) => {
                LATER.lock().remove(0);
                trigger(flags, value);
            }
            None => return,
        }
    }
}

pub fn trigger(flags: u32, value: u32) {
    let targets: Vec<(i32, SigEvent)> = NOTIFIES
        .lock()
        .iter()
        .filter(|n| n.flags & flags != 0)
        .map(|n| (n.pid, n.event))
        .collect();
    for (pid, mut ev) in targets {
        if ev.notify & event::FLAG_HANDLE == 0 {
            continue;
        }
        if ev.notify & event::FLAG_UPDATEABLE != 0 {
            ev.value = (ev.value & !0xffff_ffff) | value as u64;
        }
        if let Some(p) = process::lookup(pid) {
            let _ = event::deliver(&p, None, &ev, 0);
        }
    }
}

/// Process `pid` has ended: its notifications go, the others hear of it.
pub fn process_exited(pid: i32, daemon: bool) {
    NOTIFIES.lock().retain(|n| n.pid != pid);
    NEXT_ID.lock().remove(&pid);
    values_changed();
    trigger(PROCESS_DEATH | if daemon { DAEMON_DEATH } else { 0 }, pid as u32);
}

/// The documented hash of a path for `PATHSPACE` (ELF hash, top bit set).
pub fn pathspace_hash(path: &str) -> u32 {
    let mut hash: u32 = 0;
    for &c in path.as_bytes() {
        hash = (hash << 4).wrapping_add(c as u32);
        let x = hash & 0xf000_0000;
        if x != 0 {
            hash ^= x >> 24;
            hash &= !x;
        }
    }
    hash | 0x8000_0000
}

fn reply(rcvid: u64, status: u64) {
    let _ = call(__KER_MSG_REPLYV, [rcvid, status, 0, 0, 0, 0]);
}

/// Handle a `_PROC_EVENT` message from `sender`.
pub fn handle(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    // A message of the type and subtype only has flags 0 (as procnto).
    let flags = msg
        .get(4..8)
        .map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap()));
    let ev = || -> Result<SigEvent, u32> {
        let b: &[u8; SIGEV_SIZE] = msg.get(8..MSG_SIZE).ok_or(EINVAL)?.try_into().unwrap();
        let ev = SigEvent::from_bytes(b);
        ev.validate()?;
        Ok(ev)
    };
    match subtype {
        NOTIFY => {
            let mut n = NOTIFIES.lock();
            n.retain(|n| !(n.pid == sender && n.id == 0));
            if flags != 0 {
                n.push(Notify {
                    id: 0,
                    pid: sender,
                    flags,
                    event: ev()?,
                    watch: None,
                });
            }
            drop(n);
            reply(rcvid, 0);
        }
        NOTIFY_ADD => {
            let event = ev()?;
            let id = next_id(sender);
            NOTIFIES.lock().push(Notify {
                id,
                pid: sender,
                flags,
                event,
                watch: None,
            });
            reply(rcvid, id as u64);
        }
        NOTIFY_DEL => {
            delete(sender, flags as i32)?;
            reply(rcvid, 0);
        }
        TRIGGER | TRIGGER_UPDATEABLE => {
            // The privileged events need `PROCMGR_AID_EVENT` for each bit.
            let privileged = flags & PRIVILEGED;
            if privileged != 0 {
                let p = process::lookup(sender).ok_or(ESRCH)?;
                for bit in (16..32).map(|b| 1u32 << b).filter(|b| privileged & b != 0) {
                    if !ability::allowed(&p, ability::EVENT, Some(bit as u64)) {
                        return Err(EPERM);
                    }
                }
            }
            // A plain trigger's value is 0.
            let value = match subtype {
                TRIGGER => 0,
                _ => msg.get(24..28).map_or(0, |b| u32::from_le_bytes(b.try_into().unwrap())),
            };
            trigger(flags, value);
            reply(rcvid, 0);
        }
        _ => return Err(ENOSYS),
    }
    Ok(())
}

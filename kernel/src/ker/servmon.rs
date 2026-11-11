//! Server monitors (QNX's `server-monitor`; `_PROC_SERVMON`), as procnto's
//! `procmgr_servmonitor` and `nto_servmon_*`.
//!
//! - `_PROC_SERVER_MONITOR_CONFIG` { type, subtype, timeout (ms),
//!   target_pid, sigevent }: the sender becomes the monitor, with
//!   `PROCMGR_AID_SERVER_MONITOR` (EPERM); there is one at most (EEXIST).
//!   The event isn't checked.
//! - `_PROC_SERVER_MONITOR_IGNORE`: the monitor isn't interested in server
//!   `target_pid` (ESRCH for no such process or no monitor, EINVAL from a
//!   process that isn't the monitor; the ability too).
//! - `_PROC_SERVER_MONITOR_ALIVE`: EOK if there is a monitor, else ESRCH.
//! - A REPLY-blocked client that asks to leave a server whose channel has
//!   `_NTO_CHF_UNBLOCK` (`msg::unblock`), with a monitor and a server it
//!   doesn't ignore, is watched: if it is still waiting for the server
//!   `timeout` later, the monitor gets its event with the client's pid and
//!   tid as the value (`pid << 32 | tid`, measured) and the client's
//!   priority plus one (254 at most) as the priority.
//! - While there is a monitor, a client asking to leave a server it
//!   ignores leaves at once, its server still getting the unblock pulse
//!   (measured on procnto: as if the channel had no `_NTO_CHF_UNBLOCK`).
//! - The monitor's end unregisters it; a server's end forgets it.

use alloc::collections::{BTreeMap, BTreeSet};

use super::errno::{EEXIST, EINVAL, ESRCH};
use super::event::SigEvent;
use super::process;
use crate::clock;
use crate::sched::{State, Thread, ThreadRef};
use crate::sync::IrqSpinLock;

struct Monitor {
    pid: i32,
    /// How long a server may take to let an unblocked client go (ns).
    timeout: u64,
    event: SigEvent,
}

static MONITOR: IrqSpinLock<Option<Monitor>> = IrqSpinLock::new(None);
/// The servers the monitor isn't interested in (a process flag on
/// procnto).
static IGNORED: IrqSpinLock<BTreeSet<i32>> = IrqSpinLock::new(BTreeSet::new());
/// The clients watched: (deadline in ns of the monotonic clock, tid) ->
/// the thread and the CPU whose tick looks after it (as `timeout`).
static WATCHED: IrqSpinLock<BTreeMap<(u64, u32), (ThreadRef, u32)>> = IrqSpinLock::new(BTreeMap::new());

/// `_PROC_SERVER_MONITOR_CONFIG` from `pid` (which has the ability).
pub fn config(pid: i32, timeout_ms: u32, event: SigEvent) -> Result<(), u32> {
    let mut m = MONITOR.lock();
    if m.is_some() {
        return Err(EEXIST);
    }
    *m = Some(Monitor {
        pid,
        timeout: timeout_ms as u64 * 1_000_000,
        event,
    });
    Ok(())
}

/// `_PROC_SERVER_MONITOR_IGNORE` of server `target` from `pid`.
pub fn ignore(pid: i32, target: i32) -> Result<(), u32> {
    let m = MONITOR.lock();
    match m.as_ref() {
        None => Err(ESRCH),
        Some(m) if m.pid != pid => Err(EINVAL),
        Some(_) => {
            IGNORED.lock().insert(target);
            Ok(())
        }
    }
}

/// `_PROC_SERVER_MONITOR_ALIVE`.
pub fn alive() -> Result<(), u32> {
    if MONITOR.lock().is_some() { Ok(()) } else { Err(ESRCH) }
}

/// Process `pid` has ended: as the monitor, it is no more; as a server,
/// it is forgotten.
pub fn process_exited(pid: i32) {
    let mut m = MONITOR.lock();
    if m.as_ref().is_some_and(|m| m.pid == pid) {
        *m = None;
    }
    drop(m);
    IGNORED.lock().remove(&pid);
}

/// Whether the monitor (if there is one) ignores the server of process
/// `server`.
pub fn ignored(server: i32) -> bool {
    MONITOR.lock().is_some() && IGNORED.lock().contains(&server)
}

/// Client `t` has asked to leave the server of process `server`, which
/// decides when (`_NTO_CHF_UNBLOCK`): watch it, if there is a monitor
/// interested.
pub fn watch(t: &Thread, server: i32) {
    let timeout = match MONITOR.lock().as_ref() {
        Some(m) => m.timeout,
        None => return,
    };
    if IGNORED.lock().contains(&server) {
        return;
    }
    let deadline = clock::now_ns().saturating_add(timeout);
    let cpu = crate::percpu::this().cpu;
    let mut w = WATCHED.lock();
    w.retain(|&(_, tid), _| tid != t.tid);
    w.insert((deadline, t.tid), (t.arc(), cpu));
    drop(w);
    clock::deadline_armed(cpu, deadline);
}

/// The first deadline (ns) of the clients watched by the CPUs of `family`.
pub fn next_deadline(family: u64) -> Option<u64> {
    WATCHED.lock().iter().find(|(_, (_, cpu))| family & 1 << cpu != 0).map(|(&(d, _), _)| d)
}

/// The deadlines of the CPUs of `family` that have passed: tell the
/// monitor about the clients still waiting for their servers.
pub fn expire(family: u64) {
    let now = clock::now_ns();
    loop {
        let t = {
            let mut w = WATCHED.lock();
            let due = w
                .iter()
                .take_while(|(k, _)| k.0 <= now)
                .find(|(_, (_, cpu))| family & 1 << cpu != 0)
                .map(|(&k, _)| k);
            match due {
                Some(k) => w.remove(&k).unwrap().0,
                None => return,
            }
        };
        if t.state() == State::Reply && t.unblock_requested.load(core::sync::atomic::Ordering::Relaxed) {
            notify(&t);
        }
    }
}

/// Tell the monitor that client `t` is still waiting.
fn notify(t: &Thread) {
    let (pid, mut ev) = match MONITOR.lock().as_ref() {
        Some(m) => (m.pid, m.event),
        None => return,
    };
    ev.value = (t.process.pid as u32 as u64) << 32 | t.ptid as u64;
    ev.priority = (t.priority() as i16 + 1).min(254);
    if let Some(p) = process::lookup(pid).filter(|_| pid != 0) {
        let _ = super::event::deliver(&p, None, &ev, 0);
    }
}

//! CPU offlining, as procnto: a privileged thread stops the kernel from
//! using its CPU (to power it down, run diagnostics, ...).
//!
//! - The thread pins itself to the CPU, takes `SCHED_OFFLINING` (a FIFO
//!   policy that may go above the clock's priority, `PROCMGR_AID_RUNSTATE`,
//!   see `SchedSet()`), hands the CPU's timers to another CPU
//!   (`TimerDelegate()`), and marks the CPU offline
//!   (`SchedCtl(SCHED_PROCESSOR_OFFLINE)`). Then the reverse:
//!   `TimerDelegate(_NTO_TIMER_RECLAIM)`, `SchedCtl(SCHED_PROCESSOR_ONLINE)`.
//! - Offline, the CPU runs only `SCHED_OFFLINING` threads (`sched`), no
//!   wakeup sends it an IPI, and `SCHED_PROCESSORS_STATUS` leaves it out.
//!   Its timers and timeouts fire only if delegated: armed there, they
//!   wait until the CPU is back otherwise (as on procnto; they belong to
//!   the CPU that armed them, see `clock`).
//! - `TimerDelegate(action, owner_cpu, delegate_cpu)`: each CPU looks
//!   after a family of timer queues (`PerCpu::timer_family`), its own to
//!   begin with. `_NTO_TIMER_DELEGATE` moves the owner's whole family to
//!   the delegate, which must look after its own (EAGAIN), and stops the
//!   owner's timer; delegating again with nothing left to move succeeds
//!   doing nothing. `_NTO_TIMER_RECLAIM` takes the owner's own queue back
//!   from the delegate given (EINVAL if it doesn't have it) or, with -1,
//!   from whichever CPU has it (ESRCH if none): queues delegated to the
//!   owner meanwhile stay where they went (transitive delegation).
//!   `_NTO_TIMER_QUERY_DELEGATE` returns the CPU that has the owner's
//!   queue (ESRCH if the owner has it).
//! - procnto's checks, in its order: the action (ENOSYS); for DELEGATE
//!   and RECLAIM the delegate CPU (EINVAL: the owner itself, beyond the
//!   CPUs, -1 for DELEGATE); then a uniprocessor (ENOTSUP), the owner CPU
//!   (EINVAL), the caller's policy (EPERM), its runmask of one CPU
//!   (ENOEXEC), the owner being the CPU it runs on (ENOSYS) and its
//!   priority above the clock's (ERANGE, `procnto -C`); a concurrent call
//!   is EBUSY.
//! - `SchedCtl(SCHED_PROCESSOR_OFFLINE | ONLINE)` take or bring back the
//!   CPU of the caller's runmask: only for `SCHED_OFFLINING` threads
//!   (EPERM), with a runmask of one CPU (EINVAL), and EAGAIN if the CPU is
//!   in that state already. They don't need the timers delegated, nor a
//!   priority above the clock's.

use core::sync::atomic::{AtomicBool, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EBUSY, EINVAL, ENOEXEC, ENOSYS, ENOTSUP, EPERM, ERANGE, ESRCH};
use crate::sched::{self, Policy};
use crate::{clock, percpu};

// `TimerDelegate()` actions (`<sys/neutrino.h>`).
const TIMER_DELEGATE: u32 = 1;
const TIMER_RECLAIM: u32 = 2;
const TIMER_QUERY_DELEGATE: u32 = 3;

/// A `TimerDelegate()` is in progress (procnto's `nto_time_BFL`).
static BUSY: AtomicBool = AtomicBool::new(false);

/// The number of CPUs the system page has.
fn num_cpus() -> u32 {
    (crate::syspage::get().header().num_cpu as u32).min(percpu::MAX_CPUS as u32)
}

fn family(cpu: u32) -> &'static core::sync::atomic::AtomicU64 {
    &percpu::get(cpu).expect("a CPU of the system page").timer_family
}

/// The CPUs of the calling thread's runmask (of those there are).
fn runmask() -> u64 {
    let n = num_cpus();
    sched::current().runmask.load(Ordering::Relaxed) & ((1u64 << n) - 1)
}

/// procnto's `validate_foster_op_competency()`: may the calling thread
/// act for CPU `owner`'s timers?
fn competent(owner: u32) -> Result<(), u32> {
    let n = num_cpus();
    if n == 1 {
        return Err(ENOTSUP);
    }
    if owner >= n {
        return Err(EINVAL);
    }
    let me = sched::current();
    if me.policy() != Policy::Offlining {
        return Err(EPERM);
    }
    if runmask().count_ones() != 1 {
        return Err(ENOEXEC);
    }
    if percpu::this().cpu != owner {
        return Err(ENOSYS);
    }
    if me.priority() <= clock::ist_priority() {
        return Err(ERANGE);
    }
    Ok(())
}

/// The CPU other than `owner` that has `owner`'s timers, looking from the
/// next one round.
fn foster_of(owner: u32) -> Option<u32> {
    let n = num_cpus();
    (1..n)
        .map(|i| (owner + i) % n)
        .find(|&c| family(c).load(Ordering::Acquire) & 1 << owner != 0)
}

/// `TimerDelegate(action, owner_cpu, delegate_cpu)` (-1 owner: the
/// calling CPU).
pub fn timer_delegate(action: u32, owner: i32, delegate: i32) -> KerResult {
    let owner = if owner == -1 { percpu::this().cpu } else { owner as u32 };
    let n = num_cpus() as i32;
    match action {
        TIMER_DELEGATE | TIMER_RECLAIM => {
            let bad = if action == TIMER_DELEGATE {
                delegate as u32 >= n as u32
            } else {
                delegate >= n || delegate < -1
            };
            if delegate as u32 == owner || bad {
                return Err(EINVAL);
            }
        }
        TIMER_QUERY_DELEGATE => {}
        _ => return Err(ENOSYS),
    }
    competent(owner)?;
    if BUSY.swap(true, Ordering::Acquire) {
        return Err(EBUSY);
    }
    let r = match action {
        TIMER_DELEGATE => delegate_to(owner, delegate as u32),
        TIMER_RECLAIM => {
            let foster = if delegate == -1 { foster_of(owner) } else { Some(delegate as u32) };
            foster.ok_or(ESRCH).and_then(|f| reclaim_from(owner, f))
        }
        _ => foster_of(owner).map(u64::from).ok_or(ESRCH),
    };
    BUSY.store(false, Ordering::Release);
    r
}

/// Move `owner`'s family of timers to `to`, and stop `owner`'s timer.
fn delegate_to(owner: u32, to: u32) -> KerResult {
    if family(to).load(Ordering::Acquire) & 1 << to == 0 {
        return Err(EAGAIN);
    }
    let moved = family(owner).swap(0, Ordering::AcqRel);
    family(to).fetch_or(moved, Ordering::AcqRel);
    crate::ipi::send(to, crate::ipi::TIMERS);
    clock::family_changed();
    Ok(0)
}

/// Take `owner`'s own timers back from `from`.
fn reclaim_from(owner: u32, from: u32) -> KerResult {
    let bit = 1u64 << owner;
    if family(from).load(Ordering::Acquire) & bit == 0 {
        return Err(EINVAL);
    }
    family(from).fetch_and(!bit, Ordering::AcqRel);
    family(owner).fetch_or(bit, Ordering::AcqRel);
    crate::ipi::send(from, crate::ipi::TIMERS);
    clock::family_changed();
    Ok(0)
}

/// `SchedCtl(SCHED_PROCESSOR_ONLINE)` (`online`) or
/// `SchedCtl(SCHED_PROCESSOR_OFFLINE)`.
pub fn set_online(online: bool) -> KerResult {
    if sched::current().policy() != Policy::Offlining {
        return Err(EPERM);
    }
    let mask = runmask();
    if mask.count_ones() != 1 {
        return Err(EINVAL);
    }
    let cpu = percpu::get(mask.trailing_zeros()).expect("a CPU of the system page");
    if cpu.offline.swap(!online, Ordering::AcqRel) == !online {
        return Err(EAGAIN);
    }
    if online {
        // Threads that waited for the CPU may run there again.
        sched::request_resched();
    }
    Ok(0)
}

/// The CPUs online and not taken offline, as a bit mask.
pub fn online_mask() -> u64 {
    let mask = crate::smp::online_mask();
    (0..percpu::MAX_CPUS as u32)
        .filter(|&c| percpu::get(c).is_some_and(|p| p.offline.load(Ordering::Acquire)))
        .fold(mask, |m, c| m & !(1 << c))
}

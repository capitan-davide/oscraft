//! Clocks: `ClockTime()` and `ClockId()` (QNX `ker_clock_time`,
//! `ker_clock_id`).
//!
//! Contract, from the QNX 8.0 library reference and the SDP's procnto:
//! - `ClockId(pid, tid)` names the CPU-time clock of a process (`tid` 0)
//!   or of one of its threads; `pid` 0 is the caller. The ID is
//!   `0x4000_0000 | (pid & 0xfff) << 16 | tid`. ESRCH if there is no such
//!   process or thread.
//! - `ClockTime(id, new, old)` stores the clock's time in nanoseconds in
//!   `old` (if set). `CLOCK_REALTIME` and `CLOCK_SOFTTIME` are the time of
//!   day, `CLOCK_MONOTONIC` the time since boot, `CLOCK_PROCESS_CPUTIME_ID`
//!   and `CLOCK_THREAD_CPUTIME_ID` the caller's CPU time, and an ID from
//!   `ClockId()` that process's or thread's. EINVAL for other IDs, or a
//!   CPU-time clock whose process or thread has gone.
//! - process 1's threads 1 to the number of CPUs are the CPUs' idle threads:
//!   their clocks are how much each CPU has idled (top samples them).
//!
//! - Setting a clock (`new`): only `CLOCK_REALTIME`, by a privileged
//!   process (EPERM); it moves the system page's `qtime.nsec_tod_adjust`,
//!   which libc's `clock_gettime()` adds to the time since boot itself.
//! - `ClockAdjust(id, new, old)` (`CLOCK_REALTIME` only): add
//!   `new.tick_nsec_inc` nanoseconds to the time of day at each of the
//!   next `new.tick_count` clock ticks (0 for either stops an adjustment);
//!   `old` gets the adjustment that was in progress.

use core::sync::atomic::Ordering;
use super::KerResult;
use super::errno::{EINVAL, ENOTSUP, ESRCH};
use super::process::{self, Process};
use super::umem::{self, Space};
use crate::{clock, sched};

const CLOCK_REALTIME: u32 = 0;
const CLOCK_SOFTTIME: u32 = 1;
const CLOCK_MONOTONIC: u32 = 2;
const CLOCK_PROCESS_CPUTIME_ID: u32 = 3;
const CLOCK_THREAD_CPUTIME_ID: u32 = 4;
/// The top bits of a `ClockId()` clock.
const CPU_CLOCK: u32 = 0x4000_0000;
const CPU_CLOCK_MASK: u32 = 0xc000_0000;

/// The process whose pid's low 12 bits are `index`.
fn by_index(index: u32) -> Option<crate::ker::process::ProcessRef> {
    process::all()
        .into_iter()
        .find(|p| p.pid as u32 & 0xfff == index && !p.is_zombie())
}

/// A CPU-time clock: a process's (`tid` 0) or one of its threads'.
#[derive(Clone)]
pub enum CpuClock {
    Process(process::ProcessRef),
    Thread(process::ProcessRef, sched::ThreadRef),
}

impl CpuClock {
    /// Its time: the CPU time run, plus what setting it added
    /// (`clock_offset`).
    pub fn time(&self) -> u64 {
        match self {
            CpuClock::Process(p) => p.cpu_time().wrapping_add(p.cpu_clock_offset.load(Ordering::Relaxed)),
            CpuClock::Thread(_, t) => t.cpu_time().wrapping_add(t.cpu_clock_offset.load(Ordering::Relaxed)),
        }
    }

    fn set(&self, ns: u64) {
        match self {
            CpuClock::Process(p) => p.cpu_clock_offset.store(ns.wrapping_sub(p.cpu_time()), Ordering::Relaxed),
            CpuClock::Thread(_, t) => t.cpu_clock_offset.store(ns.wrapping_sub(t.cpu_time()), Ordering::Relaxed),
        }
    }

    fn process(&self) -> &Process {
        match self {
            CpuClock::Process(p) | CpuClock::Thread(p, _) => p,
        }
    }
}

/// The CPU-time clock `id` names, if it names one.
pub fn cpu_clock(id: u32) -> Result<Option<CpuClock>, u32> {
    Ok(Some(match id {
        CLOCK_PROCESS_CPUTIME_ID => CpuClock::Process(process::current().arc()),
        CLOCK_THREAD_CPUTIME_ID => CpuClock::Thread(process::current().arc(), sched::current().arc()),
        _ if id & CPU_CLOCK_MASK == CPU_CLOCK => {
            let p = by_index((id >> 16) & 0xfff).ok_or(EINVAL)?;
            match id & 0xffff {
                0 => CpuClock::Process(p),
                tid => {
                    let t = sched::thread_of(&p, tid).ok_or(EINVAL)?;
                    CpuClock::Thread(p, t)
                }
            }
        }
        _ => return Ok(None),
    }))
}

/// The time of clock `id` in nanoseconds.
fn time_of(id: u32) -> Result<u64, u32> {
    let now = clock::now_ns();
    match id {
        CLOCK_REALTIME | CLOCK_SOFTTIME => Ok(now.wrapping_add(clock::tod_adjust())),
        CLOCK_MONOTONIC => Ok(now),
        _ => {
            let c = cpu_clock(id)?.ok_or(EINVAL)?;
            // Another process's CPU time: the same user, or
            // PROCMGR_AID_XPROCESS_QUERY (EPERM).
            if !super::ability::may_query(process::current(), c.process()) {
                return Err(super::errno::EPERM);
            }
            Ok(c.time())
        }
    }
}

/// May the calling process set the time of day (`PROCMGR_AID_CLOCKSET`)?
fn may_set_time() -> Result<(), u32> {
    super::ability::check(super::ability::CLOCKSET, None)
}

/// `ClockTime(id, new, old)`.
pub fn clock_time(id: u32, new: u64, old: u64) -> KerResult {
    let t = time_of(id)?;
    let space = Space::current();
    // A CPU-time clock of the caller's own process may be set too (as
    // procnto; another process's: EPERM).
    if new != 0
        && let Some(c) = cpu_clock(id)?
    {
        if !core::ptr::eq(c.process(), process::current()) {
            return Err(super::errno::EPERM);
        }
        let mut b = [0u8; 8];
        umem::read(space, new, &mut b)?;
        if old != 0 {
            umem::write(space, old, &t.to_le_bytes())?;
        }
        c.set(u64::from_le_bytes(b));
        return Ok(0);
    }
    let new = if new != 0 {
        if id != CLOCK_REALTIME {
            return Err(EINVAL);
        }
        let mut b = [0u8; 8];
        umem::read(space, new, &mut b)?;
        may_set_time()?;
        Some(u64::from_le_bytes(b))
    } else {
        None
    };
    if old != 0 {
        umem::write(space, old, &t.to_le_bytes())?;
    }
    if let Some(n) = new {
        // libc adds the adjustment to the monotonic time as an unsigned
        // number: a time before the boot is the boot time (adjustment 0,
        // as procnto), not a negative one, which would read as the end
        // of time.
        clock::set_tod_adjust(n.saturating_sub(clock::now_ns()));
        crate::proc::procevent::trigger(crate::proc::procevent::TOD, 0);
    }
    Ok(0)
}

/// `ClockAdjust(id, new, old)`, with `struct _clockadjust` { tick_count,
/// tick_nsec_inc }.
pub fn clock_adjust(id: u32, new: u64, old: u64) -> KerResult {
    if id != CLOCK_REALTIME {
        return Err(EINVAL);
    }
    let space = Space::current();
    let new = if new != 0 {
        let mut b = [0u8; 8];
        umem::read(space, new, &mut b)?;
        may_set_time()?;
        Some((
            u32::from_le_bytes(b[..4].try_into().unwrap()),
            i32::from_le_bytes(b[4..].try_into().unwrap()),
        ))
    } else {
        None
    };
    let (count, inc) = clock::adjust(new);
    if old != 0 {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&count.to_le_bytes());
        b[4..].copy_from_slice(&inc.to_le_bytes());
        umem::write(space, old, &b)?;
    }
    Ok(0)
}

/// `ClockPeriod(id, new, old, reserved)`: the period of the system clock
/// (`struct _clockperiod` { nsec, fract }), its tick. It can't be changed
/// (ENOTSUP), where procnto lets a process with the CLOCKPERIOD ability
/// change it: a difference that remains (issue #301).
pub fn clock_period(id: u32, new: u64, old: u64) -> KerResult {
    time_of(id)?;
    if new != 0 {
        return Err(ENOTSUP);
    }
    if old != 0 {
        let nsec = crate::clock::tick_ns() as u32;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&nsec.to_le_bytes());
        umem::write(Space::current(), old, &b)?;
    }
    Ok(0)
}

/// `ClockId(pid, tid)`.
pub fn clock_id(pid: i32, tid: u32) -> KerResult {
    let p = if pid == 0 {
        process::current().arc()
    } else {
        process::lookup(pid)
            .filter(|p| !p.is_zombie())
            .ok_or(ESRCH)?
    };
    if tid != 0 && sched::thread_of(&p, tid).is_none() {
        return Err(ESRCH);
    }
    Ok((CPU_CLOCK | (p.pid as u32 & 0xfff) << 16 | (tid & 0xffff)) as u64)
}

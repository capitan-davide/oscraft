//! Kernel timers (QNX `ker/nto_timer.c`): timeouts for blocking calls,
//! `TimerTimeout(clock, flags, notify, ntime, otime)`, and the timers of
//! `TimerCreate()` & co. (`timer_create()`, `timer_settime()`, ...).
//!
//! `flags` holds `_NTO_TIMEOUT_*` bits, `1 << STATE_*` (`<sys/states.h>`):
//! the timeout applies to the caller's next kernel call that would block
//! in one of those states. `_NTO_TIMEOUT_NANOSLEEP` blocks at once (libc's
//! `nanosleep()`, `sleep()`): the call returns when the time is up, or
//! with EINTR and the time left in `otime` if a signal came first.
//! `TIMER_ABSTIME` in `flags` makes `ntime` an absolute time of `clock`;
//! `CLOCK_REALTIME` is the monotonic clock plus the system page's
//! `qtime.nsec_tod_adjust`. A null `ntime` disarms.
//!
//! `notify` may only be `SIGEV_UNBLOCK` (EINVAL otherwise, as procnto);
//! a CPU-time clock is ENOTSUP.
//!
//! Timers (from the QNX 8.0 library reference):
//! - `TimerCreate(clock, event)` returns the timer's id. With no event the
//!   timer sends `SIGALRM` with the id as its value; signals are sent with
//!   `si_code` `SI_TIMER`.
//! - `TimerSettime(id, flags, itime, oitime)`: `struct _itimer` is
//!   (`nsec`, `interval_nsec`); `nsec` is relative, or an absolute time of
//!   the timer's clock with `TIMER_ABSTIME`, and 0 disarms. `oitime` gets
//!   the time that was left and the old interval.
//! - `TimerInfo(pid, id, flags, info)` fills a `struct _timer_info`: the
//!   next expiry (absolute) and interval, the time left, the
//!   `_NTO_TI_*` flags, the overruns (reset with
//!   `_NTO_TIMER_RESET_OVERRUNS`) and the event; it returns the id. With
//!   `_NTO_TIMER_SEARCH`, the first timer whose id is at least `id`.
//! - A timer has one signal pending at a time (as procnto): an expiry
//!   that finds its signal still pending counts an overrun instead. A
//!   signal goes straight to a thread waiting for it. Taken by
//!   `sigwaitinfo()`, the first expiry after still counts one, and only
//!   the next sends the signal again (procnto's timing, measured).
//!   Overruns add up until `TimerInfo(_NTO_TIMER_RESET_OVERRUNS)`
//!   (`timer_getoverrun()`). Pulses are sent at each expiry.
//! - A periodic timer's next expiry is a period after the one due, or a
//!   period from now if that has passed (its clock was held back, see
//!   `clock`): missed periods aren't made up for, nor counted. A normal
//!   timer's period is at least a tick.
//! - A process's timers go away when it ends.
//! - A timer may count a CPU-time clock (a process's or a thread's, as
//!   procnto allows): it expires when that clock reaches its deadline,
//!   seen on the tick, which goes on while such a timer is armed.
//! - `TimerAlarm(clock, itime, otime)` (`alarm()`, `setitimer()`): the
//!   process's one alarm, `SIGALRM` after `nsec` (then every
//!   `interval_nsec`), reset by each call; 0 cancels it, a null `itime`
//!   only reads the time left into `otime`. Here a timer with an id
//!   `TimerCreate()` doesn't give.
//!
//! A timer belongs to the CPU that armed it (QNX 8's per-CPU timer
//! queues), or to the CPU that looks after that one's timers
//! (`TimerDelegate()`), and waits while that CPU's clock is held back
//! (`clock`). Normal timers expire on its clock tick, so their resolution
//! is a tick (QNX 8's "virtual" ticks). A high-resolution timer (HRT:
//! `TimerSettime()` with `TIMER_TOLERANCE` and a value below the tick)
//! expires on time: its CPU's timer interrupt is programmed for it. It
//! takes `PROCMGR_AID_HIGH_RESOLUTION_TIMER`.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::KerResult;
use super::errno::{EINTR, EINVAL, EPERM, ESRCH};
use super::event::{self, SigEvent};
use super::process::{self, Process, ProcessRef};
use super::umem::{self, Space};
use crate::sched::ThreadRef;
use crate::sync::IrqSpinLock;
use crate::{clock, sched, timeout};

const CLOCK_REALTIME: u32 = 0;
const CLOCK_SOFTTIME: u32 = 1;
const CLOCK_MONOTONIC: u32 = 2;
const TIMER_ABSTIME: u32 = 0x8000_0000;
/// `TIMER_TOLERANCE`: `itime` is the timer's tolerance.
const TIMER_TOLERANCE: u32 = 0x4000_0000;
/// `_NTO_TIMEOUT_NANOSLEEP` = `1 << STATE_NANOSLEEP` (12).
pub const TIMEOUT_NANOSLEEP: u32 = 1 << 12;
/// `_NTO_TIMEOUT_MASK`: the state bits.
const TIMEOUT_MASK: u32 = (1 << 24) - 1;

/// Monotonic nanoseconds of absolute time `t` on `clock`.
fn to_monotonic(clock: u32, t: u64) -> Result<u64, u32> {
    match clock {
        CLOCK_MONOTONIC => Ok(t),
        CLOCK_REALTIME | CLOCK_SOFTTIME => {
            Ok(t.saturating_sub(clock::tod_adjust()))
        }
        _ => Err(EINVAL),
    }
}

/// `TimerTimeout(clock, flags, notify, ntime, otime)`.
pub fn timer_timeout(clock_id: u32, flags: u32, notify: u64, ntime: u64, otime: u64) -> KerResult {
    let me = sched::current();
    let space = Space::current();
    // No timeouts on a CPU-time clock (procnto: ENOTSUP for a process's,
    // EINVAL for a thread's).
    match super::clock::cpu_clock(clock_id)? {
        Some(super::clock::CpuClock::Process(_)) => return Err(super::errno::ENOTSUP),
        Some(super::clock::CpuClock::Thread(..)) => return Err(EINVAL),
        None => {}
    }
    // Only SIGEV_UNBLOCK (or none) is allowed (as procnto and its docs).
    if notify != 0 && event::read(space, notify)?.kind() != event::SIGEV_UNBLOCK {
        return Err(EINVAL);
    }
    if ntime == 0 {
        // No time: with timeout states the timeout is immediate (a
        // deadline already past: the blocking call ends with ETIMEDOUT at
        // the next tick rather than at once); without, none is armed.
        me.timeout_armed.store(true, Ordering::Relaxed);
        *me.armed_timeout.lock() = match flags & TIMEOUT_MASK {
            0 => None,
            states => Some((0, states)),
        };
        return Ok(0);
    }
    let mut b = [0u8; 8];
    umem::read(space, ntime, &mut b)?;
    let t = u64::from_le_bytes(b);
    let now = clock::now_ns();
    let deadline = if flags & TIMER_ABSTIME != 0 {
        to_monotonic(clock_id, t)?
    } else {
        to_monotonic(clock_id, 0)?; // only checks the clock
        now.saturating_add(t)
    };
    if flags & TIMEOUT_NANOSLEEP == 0 {
        me.timeout_armed.store(true, Ordering::Relaxed);
        *me.armed_timeout.lock() = Some((deadline, flags & TIMEOUT_MASK));
        return Ok(0);
    }
    timeout::sleep_until(deadline);
    let now = clock::now_ns();
    if now >= deadline {
        return Ok(0);
    }
    // Woken early: a signal.
    if otime != 0 {
        umem::write(space, otime, &(deadline - now).to_le_bytes())?;
    }
    Err(EINTR)
}

const SIGALRM: i32 = 14;
const SI_USER: i32 = 0x10001;
const SI_TIMER: i32 = 0x10003;
const TI_ACTIVE: u32 = 0x1;
const TI_ABSOLUTE: u32 = 0x2;
const TI_TOD_BASED: u32 = 0x8;
const TIMER_SEARCH: u32 = 0x1;
const TIMER_RESET_OVERRUNS: u32 = 0x2;
pub const TIMER_INFO_SIZE: usize = 88;

struct Timer {
    id: i32,
    process: ProcessRef,
    thread: ThreadRef,
    clock: u32,
    event: SigEvent,
    /// Next expiry (monotonic ns), if armed.
    deadline: Option<u64>,
    interval: u64,
    absolute: bool,
    overruns: u32,
    /// A high-resolution timer (`TIMER_TOLERANCE` below the tick).
    high_res: bool,
    /// Its tolerance (`TIMER_TOLERANCE`; 0: none).
    tolerance: u64,
    /// A CPU-time clock's timer: `deadline` is a time of that clock.
    cpu_clock: Option<super::clock::CpuClock>,
    /// The CPU that armed it, whose timers it is.
    cpu: u32,
    /// Tags its signals (`SigInfo::timer`).
    key: u64,
    /// Its last signal may still be pending.
    in_flight: bool,
}

/// The next `Timer::key`.
static NEXT_KEY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

fn new_key() -> u64 {
    NEXT_KEY.fetch_add(1, Ordering::Relaxed)
}

static TIMERS: IrqSpinLock<Vec<Timer>> = IrqSpinLock::new(Vec::new());

/// Run `f` on the calling process's timer `id`.
fn with_timer<R>(id: i32, f: impl FnOnce(&mut Timer) -> R) -> Result<R, u32> {
    let me = process::current();
    let mut timers = TIMERS.lock();
    let t = timers
        .iter_mut()
        .find(|t| t.id == id && id != ALARM_ID && core::ptr::eq(&*t.process, me))
        .ok_or(EINVAL)?;
    Ok(f(t))
}

/// `TimerCreate(clock, event)`: the process's lowest free id, from 0 (as
/// procnto). Without an event, SIGALRM with the id as its value.
pub fn timer_create(clock_id: u32, ev: u64) -> KerResult {
    // A CPU-time clock (a process's or thread's), or a time clock.
    let cpu_clock = super::clock::cpu_clock(clock_id)?;
    if cpu_clock.is_none() {
        to_monotonic(clock_id, 0)?; // only checks the clock
    }
    let mut event = if ev == 0 {
        SigEvent {
            notify: event::SIGEV_SIGNAL,
            un1: SIGALRM,
            ..Default::default()
        }
    } else {
        event::read(Space::current(), ev)?
    };
    let me = sched::current();
    let mut timers = TIMERS.lock();
    let mut used: Vec<i32> = timers
        .iter()
        .filter(|t| t.id != ALARM_ID && core::ptr::eq(&*t.process, &*me.process))
        .map(|t| t.id)
        .collect();
    used.sort_unstable();
    let id = used.iter().enumerate().find(|&(i, &u)| u != i as i32).map_or(used.len(), |(i, _)| i) as i32;
    if ev == 0 {
        event.value = id as u64;
    }
    timers.push(Timer {
        id,
        process: me.process.clone(),
        thread: me.arc(),
        clock: clock_id,
        event,
        deadline: None,
        interval: 0,
        absolute: false,
        overruns: 0,
        high_res: false,
        tolerance: 0,
        cpu_clock,
        cpu: 0,
        key: new_key(),
        in_flight: false,
    });
    Ok(id as u64)
}

/// `TimerDestroy(id)`.
pub fn timer_destroy(id: i32) -> KerResult {
    let me = process::current();
    let mut timers = TIMERS.lock();
    let i = timers
        .iter()
        .position(|t| t.id == id && id != ALARM_ID && core::ptr::eq(&*t.process, me))
        .ok_or(EINVAL)?;
    timers.remove(i);
    Ok(0)
}

fn itimer(nsec: u64, interval: u64) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&nsec.to_le_bytes());
    b[8..].copy_from_slice(&interval.to_le_bytes());
    b
}

/// `TimerSettime(id, flags, itime, oitime)`.
pub fn timer_settime(id: i32, flags: u32, itime: u64, oitime: u64) -> KerResult {
    let space = Space::current();
    let mut b = [0u8; 16];
    if itime != 0 {
        umem::read(space, itime, &mut b)?;
    } else if flags & TIMER_TOLERANCE == 0 {
        // Only a tolerance can be read without setting one (procnto).
        return Err(super::errno::EFAULT);
    }
    let nsec = u64::from_le_bytes(b[..8].try_into().unwrap());
    let interval = u64::from_le_bytes(b[8..].try_into().unwrap());
    if flags & TIMER_TOLERANCE != 0 {
        // How later expiries are treated, not one: below the tick, a
        // high-resolution timer (a tolerance for low-power situations
        // otherwise, unsupported, as in QNX 8).
        // A high-resolution timer takes PROCMGR_AID_HIGH_RESOLUTION_TIMER
        // (as procnto). Without `itime`, the tolerance is only read; 0 is
        // none. The old one goes to `oitime` (its `nsec`).
        let hrt = |n: u64| n != 0 && n < clock::tick_ns();
        if itime != 0 && hrt(nsec) {
            super::ability::check(super::ability::HIGH_RESOLUTION_TIMER, None)?;
        }
        let old = with_timer(id, |t| {
            let old = t.tolerance;
            if itime != 0 {
                t.tolerance = nsec;
                t.high_res = hrt(nsec);
            }
            old
        })?;
        if oitime != 0 {
            umem::write(space, oitime, &old.to_le_bytes())?;
        }
        return Ok(0);
    }
    let now = clock::now_ns();
    let absolute = flags & TIMER_ABSTIME != 0;
    let cpu = crate::percpu::this().cpu;
    let (old, high_res) = with_timer(id, |t| -> Result<((u64, u64), bool), u32> {
        // A CPU-time timer counts in its clock's time.
        let now = t.cpu_clock.as_ref().map_or(now, |c| c.time());
        let old = (t.deadline.map_or(0, |d| d.saturating_sub(now)), t.interval);
        t.deadline = match nsec {
            0 => None,
            n if absolute && t.cpu_clock.is_some() => Some(n),
            n if absolute => Some(to_monotonic(t.clock, n)?),
            n => Some(now.saturating_add(n)),
        };
        // Disarmed, it keeps no interval (procnto reads it back as 0).
        t.interval = if nsec == 0 { 0 } else { interval };
        t.absolute = absolute;
        t.cpu = cpu;
        Ok((old, t.high_res))
    })??;
    if high_res {
        clock::hrt_armed();
    }
    if let Some(d) = deadline_of(id) {
        clock::deadline_armed(cpu, d);
    }
    if oitime != 0 {
        umem::write(space, oitime, &itimer(old.0, old.1))?;
    }
    Ok(0)
}

/// `TimerInfo(pid, id, flags, info)`; with `_NTO_TIMER_SEARCH`, the
/// first timer whose id is at least `id` (its id is returned).
pub fn timer_info(pid: i32, id: i32, flags: u32, info: u64) -> KerResult {
    let me = process::current();
    let target = match pid {
        0 => me.arc(),
        p => process::lookup(p).ok_or(ESRCH)?,
    };
    // Another process's: the same user, or PROCMGR_AID_XPROCESS_QUERY.
    if !super::ability::may_query(me, &target) {
        return Err(EPERM);
    }
    let now = clock::now_ns();
    let (id, b) = {
        let mut timers = TIMERS.lock();
        let t = timers
            .iter_mut()
            .filter(|t| t.id != ALARM_ID && core::ptr::eq(&*t.process, &*target))
            .filter(|t| if flags & TIMER_SEARCH != 0 { t.id >= id } else { t.id == id })
            .min_by_key(|t| t.id)
            .ok_or(EINVAL)?;
        let b = info_of(t, now);
        if flags & TIMER_RESET_OVERRUNS != 0 {
            t.overruns = 0;
        }
        (t.id, b)
    };
    umem::write(Space::current(), info, &b)?;
    Ok(id as u64)
}

/// The `struct _timer_info` of `t`: the time left counts down to the
/// deadline as procnto's (a past one reads as a negative time, wrapped).
fn info_of(t: &Timer, now: u64) -> [u8; TIMER_INFO_SIZE] {
    let mut b = [0u8; TIMER_INFO_SIZE];
    let now = t.cpu_clock.as_ref().map_or(now, |c| c.time());
    let adjust = match t.clock {
        CLOCK_MONOTONIC => 0,
        _ if t.cpu_clock.is_some() => 0,
        _ => clock::tod_adjust(),
    };
    let mut ti_flags = 0;
    if t.deadline.is_some() {
        ti_flags |= TI_ACTIVE;
    }
    if t.absolute {
        ti_flags |= TI_ABSOLUTE;
    }
    if t.clock != CLOCK_MONOTONIC {
        ti_flags |= TI_TOD_BASED;
    }
    // Modulo 2^64, as procnto (measured, with the time of day set near
    // the top of the range) and as `clock_time` adds it.
    let next = t.deadline.map_or(0, |d| d.wrapping_add(adjust));
    let left = t.deadline.map_or(0, |d| d.wrapping_sub(now));
    b[0..16].copy_from_slice(&itimer(next, t.interval));
    b[16..32].copy_from_slice(&itimer(left, t.interval));
    b[32..36].copy_from_slice(&ti_flags.to_le_bytes());
    b[40..44].copy_from_slice(&(t.event.notify & 0xffff).to_le_bytes());
    b[44..48].copy_from_slice(&t.clock.to_le_bytes());
    b[48..52].copy_from_slice(&t.overruns.to_le_bytes());
    b[56..60].copy_from_slice(&t.event.notify.to_le_bytes());
    b[64..68].copy_from_slice(&t.event.un1.to_le_bytes());
    b[72..80].copy_from_slice(&t.event.value.to_le_bytes());
    b[80..82].copy_from_slice(&t.event.code.to_le_bytes());
    b[82..84].copy_from_slice(&t.event.priority.to_le_bytes());
    b
}

/// The timers of `p` (not its alarm), by id: their ids and `struct
/// _timer_info`s (`DCMD_PROC_TIMERS`).
pub fn timers_of(p: &Process) -> Vec<(i32, [u8; TIMER_INFO_SIZE])> {
    let now = clock::now_ns();
    let mut v: Vec<_> = TIMERS
        .lock()
        .iter()
        .filter(|t| t.id != ALARM_ID && core::ptr::eq(&*t.process, p))
        .map(|t| (t.id, info_of(t, now)))
        .collect();
    v.sort_by_key(|(id, _)| *id);
    v
}

/// The id of a process's alarm (not one `TimerCreate()` gives).
const ALARM_ID: i32 = -1;

/// `TimerAlarm(clock, itime, otime)`.
pub fn timer_alarm(clock_id: u32, itime: u64, otime: u64) -> KerResult {
    to_monotonic(clock_id, 0)?; // only checks the clock
    let me = process::current();
    let space = Space::current();
    let mut b = [0u8; 16];
    if itime != 0 {
        umem::read(space, itime, &mut b)?;
    }
    let nsec = u64::from_le_bytes(b[..8].try_into().unwrap());
    let interval = u64::from_le_bytes(b[8..].try_into().unwrap());
    let now = clock::now_ns();
    let old = {
        let mut timers = TIMERS.lock();
        let i = timers
            .iter()
            .position(|t| t.id == ALARM_ID && core::ptr::eq(&*t.process, me));
        let old = i.map_or((0, 0), |i| {
            let t = &timers[i];
            (t.deadline.map_or(0, |d| d.saturating_sub(now)), t.interval)
        });
        if itime != 0 {
            if let Some(i) = i {
                timers.remove(i);
            }
            if nsec != 0 {
                timers.push(Timer {
                    id: ALARM_ID,
                    process: me.arc(),
                    thread: sched::current().arc(),
                    clock: clock_id,
                    event: SigEvent {
                        notify: event::SIGEV_SIGNAL,
                        un1: SIGALRM,
                        ..Default::default()
                    },
                    deadline: Some(now.saturating_add(nsec)),
                    interval,
                    absolute: false,
                    overruns: 0,
                    high_res: false,
                    tolerance: 0,
                    cpu_clock: None,
                    cpu: crate::percpu::this().cpu,
                    key: new_key(),
                    in_flight: false,
                });
            }
        }
        old
    };
    if otime != 0 {
        umem::write(space, otime, &itimer(old.0, old.1))?;
    }
    Ok(0)
}

/// The `si_code` of timer `t`'s signal: SI_TIMER, but 0 for the alarm
/// (as procnto sends `alarm()`'s SIGALRM).
fn code_of(t: &Timer) -> i32 {
    if t.id == ALARM_ID { SI_USER } else { SI_TIMER }
}

/// What a timer's expiry delivers.
type Fired = (ProcessRef, ThreadRef, SigEvent, i32, u64);

/// Timer `t`, due, expires at `now` (of its clock): its next deadline,
/// and what to deliver (`deliver` decides whether it is sent).
fn fire(t: &mut Timer, now: u64) -> Option<Fired> {
    let d = t.deadline?;
    t.deadline = if t.interval == 0 {
        None
    } else {
        // procnto's `set_new_fire_time()`.
        let interval = if t.high_res { t.interval } else { t.interval.max(clock::tick_ns()) };
        let next = d.saturating_add(interval);
        Some(if next < now { now.saturating_add(interval) } else { next })
    };
    Some((t.process.clone(), t.thread.clone(), t.event, code_of(t), t.key))
}

/// Deliver the events of expired timers. A signal goes to a thread that
/// waits for it (`sigwaitinfo()`) as it comes; otherwise it is queued,
/// one per timer: while it is (`Timer::in_flight`), expiries count
/// overruns. Delivered to a handler, it frees the timer at once
/// (`signal_taken`); taken by `sigwaitinfo()`, the timer only learns of
/// it at the expiry after, which still counts one (as procnto).
fn deliver(fired: Vec<Fired>) {
    for (p, t, ev, code, key) in fired {
        let signal = ev.notify & event::FLAG_HANDLE == 0
            && matches!(ev.kind(), event::SIGEV_SIGNAL | event::SIGEV_SIGNAL_CODE | event::SIGEV_SIGNAL_THREAD);
        if signal {
            let only = (ev.kind() == event::SIGEV_SIGNAL_THREAD).then_some(&*t);
            let waited = super::signal::waited_for(&p, p.personality().signo_in(ev.un1 as usize), only);
            let queued = p.signals.lock().from_timer(key);
            let send = {
                let mut timers = TIMERS.lock();
                let Some(tm) = timers.iter_mut().find(|tm| tm.key == key) else { continue };
                if waited {
                    tm.in_flight = false;
                    true
                } else if tm.in_flight {
                    tm.overruns = tm.overruns.saturating_add(1);
                    tm.in_flight = queued;
                    false
                } else {
                    tm.in_flight = true;
                    true
                }
            };
            if !send {
                continue;
            }
        }
        let _ = event::deliver_from(&p, Some(t), &ev, code, key);
    }
}

/// Fire the normal timers of the CPUs of `family` (bit n: CPU n's) whose
/// time has come (from the clock tick of the CPU that looks after them).
pub fn expire(family: u64) {
    let now = clock::now_ns();
    let mut fired = Vec::new();
    {
        let mut timers = TIMERS.lock();
        for t in timers.iter_mut().filter(|t| !t.high_res && family & 1 << t.cpu != 0) {
            let now = t.cpu_clock.as_ref().map_or(now, |c| c.time());
            if t.deadline.is_some_and(|d| d <= now) {
                fired.extend(fire(t, now));
            }
        }
    }
    deliver(fired);
}

/// The signal of the timer tagged `key` (`SigInfo::timer`) was delivered
/// (to a handler, or ignored on its way there): the timer sends the next
/// at its next expiry (procnto unlinks it from the timer then).
pub fn signal_taken(key: u64) {
    if let Some(t) = TIMERS.lock().iter_mut().find(|t| t.key == key) {
        t.in_flight = false;
    }
}

/// The deadline of timer `id`, if armed (a CPU-time timer's isn't a time
/// of day: none).
fn deadline_of(id: i32) -> Option<u64> {
    with_timer(id, |t| if t.cpu_clock.is_some() { clock::now_ns().checked_add(clock::tick_ns()) } else { t.deadline })
        .ok()
        .flatten()
}

/// The first deadline (monotonic ns) of the normal timers of the CPUs of
/// `family`.
pub fn next_deadline(family: u64) -> Option<u64> {
    let timers = TIMERS.lock();
    let normal = timers.iter().filter(|t| !t.high_res && family & 1 << t.cpu != 0);
    // A CPU-time timer's deadline isn't a time of day: while one is armed
    // the tick goes on, to see its clock reach it.
    if normal.clone().any(|t| t.cpu_clock.is_some() && t.deadline.is_some()) {
        return Some(clock::now_ns().saturating_add(clock::tick_ns()));
    }
    normal.filter_map(|t| t.deadline).min()
}

/// Expire the due high-resolution timers of the CPUs of `family` (bit n:
/// CPU n's; on the timer interrupt of the CPU that looks after them);
/// returns the next deadline (monotonic ns) of the ones left armed.
pub fn expire_hrt(family: u64) -> Option<u64> {
    let now = clock::now_ns();
    let mut fired = Vec::new();
    let next = {
        let mut timers = TIMERS.lock();
        let mut next: Option<u64> = None;
        for t in timers.iter_mut().filter(|t| t.high_res && family & 1 << t.cpu != 0) {
            if t.deadline.is_some_and(|d| d <= now) {
                fired.extend(fire(t, now));
            }
            if let Some(d) = t.deadline {
                next = Some(next.map_or(d, |n| n.min(d)));
            }
        }
        next
    };
    deliver(fired);
    next
}

/// Remove the timers of a process that has ended.
pub fn process_exited(p: &Process) {
    TIMERS.lock().retain(|t| !core::ptr::eq(&*t.process, p));
}

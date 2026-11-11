//! The system clock tick, and high-resolution timers.
//!
//! Each CPU's timer interrupt comes at its next tick or at its next
//! high-resolution timer expiry (`ker::timer`), whichever is first: the
//! tick's work is done only on a tick.
//!
//! Timeouts and timers belong to the CPU that armed them (QNX 8's per-CPU
//! timer queues): its tick expires its timeouts and normal timers, its
//! timer interrupt is programmed for its high-resolution timers. A CPU
//! looks after the timers of its "family" (`PerCpu::timer_family`): its
//! own, unless it delegated them to another CPU (`TimerDelegate()`,
//! `ker::offline`), and those delegated to it. The CPU whose family has
//! CPU 0 also does the system's work on its tick (clock adjustment,
//! sporadic replenishments, trace buffers); here "CPU 0" below. A CPU
//! with no family has no tick, as procnto stops the timer of a CPU that
//! delegated its own.
//!
//! procnto does a CPU's clock work (ticks, timers) in an interrupt thread
//! at the clock's priority (`procnto -C`, 254 by default), so a thread
//! at or above it (`SCHED_OFFLINING`, `SCHED_HIGH_PRIORITY_IST`) holds the
//! work back on its CPU until the CPU runs at a lower priority: timers
//! and timeouts armed there wait for it. OsCraft does it in the
//! interrupt, but holds it back the same way (`PerCpu::clock_held`,
//! `priority_lowered`).

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use crate::arch::timer;
use crate::interrupt;
use crate::percpu;
use crate::syspage::Syspage;

/// Ticks per second: procnto's `-C` (1000, a 1 ms tick, by default).
static TICK_HZ: AtomicU64 = AtomicU64::new(1000);
/// The tick in nanoseconds (of the period the counter can give).
static TICK_NS: AtomicU64 = AtomicU64::new(1_000_000);
/// The priority of procnto's clock interrupt thread (`-C`'s second
/// value, 254 by default). OsCraft has no such thread (the tick's work is
/// done in the interrupt), but the priority bounds others': threads of
/// the normal policies run below it, offlining ones may go above.
static IST_PRIO: AtomicU8 = AtomicU8::new(254);

/// Ticks per second.
pub fn tick_hz() -> u64 {
    TICK_HZ.load(Ordering::Relaxed)
}

/// The tick in nanoseconds.
pub fn tick_ns() -> u64 {
    TICK_NS.load(Ordering::Relaxed)
}

/// The clock interrupt thread's priority.
pub fn ist_priority() -> u8 {
    IST_PRIO.load(Ordering::Relaxed)
}

static PERIOD: AtomicU64 = AtomicU64::new(0);
/// The counter value of CPU 0's first tick: every CPU's ticks fall on
/// this plus multiples of the period, idle or not (QNX's virtual ticks,
/// on one grid: a thread that sleeps on one CPU and wakes on another
/// still sleeps whole ticks).
static PHASE: AtomicU64 = AtomicU64::new(0);
/// `PerCpu::idle_until` while the CPU ticks.
const TICKING: u64 = 0;
/// CPU 0's timer vector (`qtime.intr`); each CPU has its own instance.
static VECTOR0: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct ClockInfo {
    pub counter_hz: u64,
    pub vector: u32,
}

/// Set up the clock from the system page and procnto's `-C` (`hz`,
/// `ist_prio`), and start the tick on CPU 0.
pub fn init(sp: &Syspage, hz: u32, ist_prio: u8) -> Result<ClockInfo, &'static str> {
    let q = sp.qtime();
    let counter_hz = timer::frequency();
    if q.cycles_per_sec != counter_hz {
        return Err("qtime.cycles_per_sec differs from the counter frequency");
    }
    let vector = u32::try_from(q.intr).map_err(|_| "no timer interrupt in qtime")?;
    // The closest period the counter gives (as procnto: 8333333 Hz can't
    // make 1 kHz, the tick is 999960 ns then).
    let period = (q.cycles_per_sec + hz as u64 / 2) / hz as u64;
    let tick_ns = (period as u128 * 1_000_000_000 / q.cycles_per_sec as u128) as u64;
    PERIOD.store(period, Ordering::Relaxed);
    TICK_HZ.store(hz as u64, Ordering::Relaxed);
    TICK_NS.store(tick_ns, Ordering::Relaxed);
    IST_PRIO.store(ist_prio, Ordering::Relaxed);
    // The monotonic clock starts now, as procnto's: qtime's `boot_cc` is
    // the counter at the kernel's start (libc reads the clocks from it).
    let boot_cc = timer::counter();
    BOOT_CC.store(boot_cc, Ordering::Relaxed);
    VECTOR0.store(vector as u64, Ordering::Relaxed);
    // What procnto fills in: the tick in nanoseconds (libc rounds
    // timer_gettime()'s values to it) and in counter cycles,
    // QTIME_FLAG_CHECK_STABLE, and no limit on the timer's comparator.
    const QTIME_FLAG_CHECK_STABLE: u32 = 0x2;
    let qp = sp.qtime_ptr();
    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "each `(&raw mut (*q).field)` projection and its volatile access are one access to one field"
    )]
    // SAFETY: fields of the system page's qtime section, before any
    // process reads them.
    unsafe {
        (&raw mut (*qp).nsec_inc).write_volatile(tick_ns as u32);
        (&raw mut (*qp).tick_period_cc).write_volatile(period);
        (&raw mut (*qp).flags).write_volatile(q.flags | QTIME_FLAG_CHECK_STABLE);
        (&raw mut (*qp).timer_load_max).write_volatile(u64::MAX);
        (&raw mut (*qp).boot_cc).write_volatile(boot_cc);
    }
    start_cpu()?;
    Ok(ClockInfo { counter_hz, vector })
}

/// Start the tick on the calling CPU (after `init`).
pub fn start_cpu() -> Result<(), &'static str> {
    let vector =
        interrupt::local_vector(VECTOR0.load(Ordering::Relaxed) as u32).ok_or("no timer vector")?;
    let deadline = if percpu::this().cpu == 0 {
        let d = timer::counter() + PERIOD.load(Ordering::Relaxed);
        PHASE.store(d, Ordering::Relaxed);
        d
    } else {
        tick_at_or_after(timer::counter() + 1)
    };
    percpu::this().deadline.store(deadline, Ordering::Relaxed);
    interrupt::attach(vector, tick)?;
    timer::arm_at(deadline);
    Ok(())
}

// ANCHOR: clock_tick
/// The timer interrupt of a CPU: its tick (if due), its high-resolution
/// timers, then the next of both.
fn tick(_vector: u32) {
    let cpu = percpu::this();
    if hold(cpu) {
        return;
    }
    let now = timer::counter();
    let due = cpu.deadline.load(Ordering::Relaxed);
    if now >= due {
        cpu.ticks.fetch_add(1, Ordering::Relaxed);
        let period = PERIOD.load(Ordering::Relaxed);
        let mut next = due + period;
        if next <= now {
            // Late (e.g. the host descheduled the emulator): skip, don't burst.
            next = now + period;
        }
        let family = cpu.timer_family.load(Ordering::Acquire);
        cpu.deadline.store(if family == 0 { u64::MAX } else { next }, Ordering::Relaxed);
        if family & 1 != 0 {
            adjust_tick();
            crate::sporadic::expire();
            crate::ker::trace::kick();
        }
        if family != 0 {
            crate::timeout::expire(family);
            crate::ker::timer::expire(family);
            crate::ker::servmon::expire(family);
        }
        crate::sched::tick();
    }
    hrt_changed();
}
// ANCHOR_END: clock_tick

/// Stop the calling CPU's tick while it idles (tickless idle, as QNX 8):
/// it keeps the tick at which the first timeout or normal timer it looks
/// after is due (still on the tick grid, so they fire as on a tick),
/// none if none is, and its high-resolution timers interrupt it; IPIs
/// wake it for work. CPU 0 keeps ticking while a clock adjustment is in
/// progress. With interrupts masked; `restart_tick` when it wakes.
/// Returns false if the CPU should not wait (a deadline came meanwhile).
pub fn stop_tick() -> bool {
    let cpu = percpu::this();
    if holds_cpu0(cpu) && ADJUST.lock().0 != 0 {
        return true; // keeps ticking, and waits
    }
    let next = next_due(cpu);
    cpu.idle_until.store(next, Ordering::SeqCst);
    // Against a timer armed meanwhile on a CPU that delegated its timers
    // here, which read `idle_until` as TICKING: look again, after
    // publishing.
    core::sync::atomic::fence(Ordering::SeqCst);
    if next_due(cpu) < next {
        cpu.idle_until.store(TICKING, Ordering::Relaxed);
        return false;
    }
    cpu.deadline.store(next, Ordering::Relaxed);
    arm();
    true
}

/// The tick at which `cpu` next has work: the first tick of the grid at
/// or after the first timeout or normal timer of its family, or of the
/// system's work if it holds CPU 0's (`u64::MAX`: none).
fn next_due(cpu: &percpu::PerCpu) -> u64 {
    let family = cpu.timer_family.load(Ordering::Acquire);
    if family == 0 {
        return u64::MAX;
    }
    let first = [
        crate::timeout::next_deadline(family),
        crate::ker::timer::next_deadline(family),
        crate::ker::servmon::next_deadline(family),
        if holds_cpu0(cpu) { crate::sporadic::next_deadline() } else { None },
    ]
        .into_iter()
        .flatten()
        .min();
    first.map_or(u64::MAX, |ns| tick_at_or_after(ns_to_counter(ns)))
}

/// The first tick at or after counter value `c`, or `u64::MAX` if there is
/// none in range: a timer set to expire at the end of time (`tv_sec` -1
/// saturates its deadline) gets no tick (#282).
fn tick_at_or_after(c: u64) -> u64 {
    let (phase, period) = (PHASE.load(Ordering::Relaxed), PERIOD.load(Ordering::Relaxed));
    if c <= phase {
        return phase;
    }
    (c - phase)
        .div_ceil(period)
        .checked_mul(period)
        .and_then(|d| d.checked_add(phase))
        .unwrap_or(u64::MAX)
}

// ANCHOR: deadline_armed
/// A timeout or normal timer of CPU `owner` (the calling CPU's, or CPU
/// 0's for the system's work) is now due at monotonic time `ns`: wake the
/// CPU that looks after them if it idles past then. Call after queueing
/// it.
pub fn deadline_armed(owner: u32, ns: u64) {
    core::sync::atomic::fence(Ordering::SeqCst);
    let holder = holder_of(owner);
    let Some(h) = percpu::get(holder) else { return };
    let until = h.idle_until.load(Ordering::SeqCst);
    if until != TICKING && ns_to_counter(ns) < until {
        crate::ipi::send(holder, crate::ipi::RESCHED);
    }
}
// ANCHOR_END: deadline_armed

/// Does `cpu` look after CPU 0's timers (the system's)?
fn holds_cpu0(cpu: &percpu::PerCpu) -> bool {
    cpu.timer_family.load(Ordering::Acquire) & 1 != 0
}

/// The CPU looking after CPU `c`'s timers (`c` if none: no family has it
/// while it moves).
pub fn holder_of(c: u32) -> u32 {
    (0..percpu::MAX_CPUS as u32)
        .find(|&h| percpu::get(h).is_some_and(|p| p.timer_family.load(Ordering::Acquire) & 1 << c != 0))
        .unwrap_or(c)
}

/// The calling CPU's family of timers has changed (`ker::offline`):
/// start or stop its tick, and expire or program its high-resolution
/// timers.
pub fn family_changed() {
    let daif = crate::arch::irq_save();
    restart_tick();
    hrt_changed();
    crate::arch::irq_restore(daif);
}

/// Start the calling CPU's tick again, at the next tick of the grid. A
/// tick that is due (what woke the CPU) stays, for the timer interrupt
/// that follows to do its work.
pub fn restart_tick() {
    let cpu = percpu::this();
    let now = timer::counter();
    let due = cpu.deadline.load(Ordering::Relaxed);
    let deadline = if cpu.timer_family.load(Ordering::Acquire) == 0 {
        u64::MAX
    } else if due <= now {
        due
    } else {
        tick_at_or_after(now + 1)
    };
    cpu.idle_until.store(TICKING, Ordering::Relaxed);
    cpu.deadline.store(deadline, Ordering::Relaxed);
    arm();
}

/// The calling CPU's high-resolution timers have changed (or expired):
/// expire the due ones and program the timer interrupt for the next of
/// them or the next tick. Call with interrupts masked, or from the timer
/// interrupt.
pub fn hrt_changed() {
    let daif = crate::arch::irq_save();
    let cpu = percpu::this();
    if hold(cpu) {
        crate::arch::irq_restore(daif);
        return;
    }
    let family = cpu.timer_family.load(Ordering::Acquire);
    let next = crate::ker::timer::expire_hrt(family).map_or(u64::MAX, ns_to_counter);
    cpu.hrt_deadline.store(next, Ordering::Relaxed);
    arm();
    crate::arch::irq_restore(daif);
}

/// Hold the clock's work back if `cpu` (the calling CPU) runs a thread at
/// or above the clock's priority: its timer interrupt is silenced until
/// `priority_lowered`. With interrupts masked.
fn hold(cpu: &percpu::PerCpu) -> bool {
    if cpu.running_prio.load(Ordering::Relaxed) < ist_priority() {
        return false;
    }
    cpu.clock_held.store(true, Ordering::Relaxed);
    timer::arm_at(u64::MAX);
    true
}

/// The calling CPU may now run at a lower priority: the clock's work
/// held back (`hold`) is done now, in a timer interrupt that comes at
/// once (once interrupts are unmasked).
pub fn priority_lowered() {
    let cpu = percpu::this();
    if cpu.clock_held.load(Ordering::Relaxed) && cpu.running_prio.load(Ordering::Relaxed) < ist_priority() {
        cpu.clock_held.store(false, Ordering::Relaxed);
        timer::arm_at(0);
    }
}

/// A high-resolution timer of the calling CPU was armed: program its
/// timer interrupt, or that of the CPU its timers are delegated to.
pub fn hrt_armed() {
    hrt_changed();
    let me = percpu::this().cpu;
    let holder = holder_of(me);
    if holder != me {
        crate::ipi::send(holder, crate::ipi::TIMERS);
    }
}

/// Program the calling CPU's timer interrupt for the first of its tick
/// and its next high-resolution timer expiry. With interrupts masked.
fn arm() {
    let cpu = percpu::this();
    timer::arm_at(cpu.deadline.load(Ordering::Relaxed).min(cpu.hrt_deadline.load(Ordering::Relaxed)));
}

/// The counter value at monotonic time `ns` (rounded up).
fn ns_to_counter(ns: u64) -> u64 {
    if ns == u64::MAX {
        return u64::MAX;
    }
    let cc = (ns as u128 * timer::frequency() as u128).div_ceil(1_000_000_000) as u64;
    cc.saturating_add(BOOT_CC.load(Ordering::Relaxed))
}

/// The time of day: nanoseconds since boot plus this (the system page's
/// `qtime.nsec_tod_adjust`, which libc's `clock_gettime()` reads too).
#[expect(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each `(&raw mut (*q).field)` projection and its volatile access are one access to one field"
)]
pub fn tod_adjust() -> u64 {
    let q = crate::syspage::get().qtime_ptr();
    // SAFETY: a field of the system page's qtime section.
    unsafe { (&raw const (*q).nsec_tod_adjust).read_volatile() }
}

#[expect(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each `(&raw mut (*q).field)` projection and its volatile access are one access to one field"
)]
pub fn set_tod_adjust(v: u64) {
    let q = crate::syspage::get().qtime_ptr();
    // SAFETY: as above.
    unsafe { (&raw mut (*q).nsec_tod_adjust).write_volatile(v) }
}

/// The time of day as nanoseconds since the epoch.
pub fn tod_ns() -> u64 {
    now_ns().wrapping_add(tod_adjust())
}

/// `ClockAdjust()`'s adjustment in progress: ticks left, nanoseconds added
/// to the time of day at each (also in the system page's `qtime.adjust`).
static ADJUST: crate::sync::IrqSpinLock<(u32, i32)> = crate::sync::IrqSpinLock::new((0, 0));

/// Replace the adjustment in progress (with `new`, if given); returns the
/// previous one.
pub fn adjust(new: Option<(u32, i32)>) -> (u32, i32) {
    let mut a = ADJUST.lock();
    let old = *a;
    if let Some(n) = new {
        *a = if n.0 == 0 || n.1 == 0 { (0, 0) } else { n };
        publish_adjust(*a);
    }
    old
}

#[expect(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each `(&raw mut (*q).field)` projection and its volatile access are one access to one field"
)]
fn publish_adjust((count, inc): (u32, i32)) {
    let q = crate::syspage::get().qtime_ptr();
    // SAFETY: fields of the system page's qtime section.
    unsafe {
        (&raw mut (*q).adjust_tick_count).write_volatile(count);
        (&raw mut (*q).adjust_tick_nsec_inc).write_volatile(inc);
    }
}

/// A tick of an adjustment in progress (on CPU 0's tick).
fn adjust_tick() {
    let mut a = ADJUST.lock();
    if a.0 == 0 {
        return;
    }
    set_tod_adjust(tod_adjust().saturating_add_signed(a.1 as i64));
    a.0 -= 1;
    if a.0 == 0 {
        a.1 = 0;
    }
    publish_adjust(*a);
}

/// The counter value at which the monotonic clock started (`init`).
static BOOT_CC: AtomicU64 = AtomicU64::new(0);

// ANCHOR: now_ns
/// The monotonic clock: nanoseconds since the kernel's start (qtime's
/// `boot_cc`). Exact (rounded down), the inverse of `ns_to_counter`
/// (rounded up): at the counter value a deadline was armed for, the
/// clock reads at least that deadline, so a timer never slips a tick
/// (review CORE-19).
pub fn now_ns() -> u64 {
    let f = timer::frequency();
    let cc = timer::counter().saturating_sub(BOOT_CC.load(Ordering::Relaxed));
    // Whole seconds, then the rest of a second: two 64-bit divisions.
    (cc / f) * 1_000_000_000 + (cc % f) * 1_000_000_000 / f
}
// ANCHOR_END: now_ns

/// Ticks taken on `cpu`.
pub fn ticks(cpu: u32) -> u64 {
    percpu::get(cpu).map_or(0, |c| c.ticks.load(Ordering::Relaxed))
}

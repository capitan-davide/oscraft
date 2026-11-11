//! Sporadic scheduling (`SCHED_SPORADIC`), as procnto does it.
//!
//! A sporadic thread runs at its normal priority for at most its initial
//! budget of run time, then at its low priority until the budget is
//! replenished, one replenishment period after the run began. From
//! procnto's `replenish_sched` & co. (interface facts only):
//! - Its budget starts when it first runs: the budget ends at its run time
//!   then plus `init_budget` (its "deadline"), and the run's activation
//!   time is now.
//! - Each time it blocks at its normal priority, the run time it used
//!   since the last such point is one more chunk: the first chunk schedules
//!   a replenishment at activation + `repl_period` of that amount; later
//!   ones add to it (one pending replenishment per thread).
//! - It drops to its low priority when it uses up its budget, or when the
//!   number of chunks would reach `max_repl`.
//! - When it unblocks at its normal priority, a new activation starts.
//! - At the replenishment: if it dropped, it is raised back to its normal
//!   priority with a fresh budget; else its budget grows by the amount.
//!
//! The budget's end is a deadline in the thread's run time, looked at on
//! the ticks of the CPU that runs it (as round-robin timeslices are).
//! Replenishments are due on the tick of the CPU that looks after CPU 0's
//! timers (`clock`).
//!
//! `SchedSet(SCHED_SPORADIC)` checks (`nto_sched_sporadic_validate`):
//! `max_repl` from 1 to 65535, a low priority from 1 and below the normal
//! one, and a budget no longer than the period; EINVAL otherwise.

use alloc::collections::BTreeMap;

use crate::clock;
use crate::sched::{self, Thread, ThreadRef};
use crate::sync::IrqSpinLock;

/// A sporadic thread's state.
#[derive(Clone, Copy, Default)]
pub struct SsInfo {
    pub low_priority: u8,
    pub max_repl: u16,
    /// Replenishment period and initial budget, in ns.
    pub repl_period: u64,
    pub init_budget: u64,
    /// Its normal priority while it runs at its low one; 0 while it runs
    /// at its normal one.
    normal_prio: u8,
    /// Its run time at which its budget ends: 0 before its run begins,
    /// `u64::MAX` for none.
    deadline: u64,
    /// Its run time at the last chunk's start.
    act_runtime: u64,
    /// When its run (activation) began (monotonic ns).
    activation: u64,
    /// Chunks used since the last replenishment.
    repl_count: u16,
    /// The pending replenishment: its amount and when (monotonic ns).
    amount: u64,
    repl_at: Option<u64>,
}

/// The parameters of `struct sched_param`'s sporadic part.
#[derive(Clone, Copy)]
pub struct Params {
    pub low_priority: i32,
    pub max_repl: i32,
    pub repl_period: (i64, i64),
    pub init_budget: (i64, i64),
}

/// `timespec` to ns, as procnto (an overflow is `u64::MAX`, a negative
/// time 0).
fn ns_of((sec, nsec): (i64, i64)) -> u64 {
    if sec < 0 || (sec == 0 && nsec < 0) {
        return 0;
    }
    (sec as u64)
        .checked_mul(1_000_000_000)
        .and_then(|s| s.checked_add_signed(nsec))
        .unwrap_or(u64::MAX)
}

/// Check the parameters for normal priority `prio` (EINVAL as procnto).
pub fn validate(prio: u8, p: &Params) -> Result<(), u32> {
    use crate::ker::errno::EINVAL;
    if !(1..=0xffff).contains(&p.max_repl) || p.low_priority < 1 || p.low_priority >= prio as i32 {
        return Err(EINVAL);
    }
    if ns_of(p.repl_period) < ns_of(p.init_budget) {
        return Err(EINVAL);
    }
    Ok(())
}

/// Pending replenishments: (when, tid) -> thread.
static QUEUE: IrqSpinLock<BTreeMap<(u64, u32), ThreadRef>> = IrqSpinLock::new(BTreeMap::new());

/// Make `t` sporadic at normal priority `prio` with parameters `p`
/// (validated). Returns the priority it is to run at.
pub fn set(t: &Thread, prio: u8, p: &Params) -> u8 {
    let mut ss = t.ss.lock();
    let s = ss.get_or_insert_with(SsInfo::default);
    s.low_priority = p.low_priority as u8;
    s.max_repl = p.max_repl as u16;
    s.repl_period = ns_of(p.repl_period);
    s.init_budget = ns_of(p.init_budget);
    if s.normal_prio != 0 {
        s.normal_prio = prio;
        s.low_priority
    } else {
        prio
    }
}

/// `t` is no longer sporadic.
pub fn clear(t: &Thread) {
    if let Some(s) = t.ss.lock().take()
        && let Some(at) = s.repl_at
    {
        QUEUE.lock().remove(&(at, t.tid));
    }
}

/// `t`'s parameters, and its normal priority if it runs at its low one.
pub fn get(t: &Thread) -> Option<(SsInfo, Option<u8>)> {
    let ss = t.ss.lock();
    ss.map(|s| (s, (s.normal_prio != 0).then_some(s.normal_prio)))
}

/// `SchedSet(SCHED_SETPRIO)` on sporadic thread `t`: its normal priority,
/// if it runs at its low one, is `prio` too.
pub fn set_prio(t: &Thread, prio: u8) {
    if let Some(s) = t.ss.lock().as_mut()
        && s.normal_prio != 0
    {
        s.normal_prio = prio;
    }
}

/// Sporadic thread `t` starts running at monotonic time `now`, with `ran`
/// ns of run time: its budget starts, if it hasn't.
pub fn start(t: &Thread, now: u64, ran: u64) {
    if let Some(s) = t.ss.lock().as_mut() {
        if s.normal_prio != 0 {
            s.deadline = u64::MAX;
        } else if s.deadline == 0 {
            s.act_runtime = ran;
            s.deadline = ran.saturating_add(s.init_budget);
            s.activation = now;
        }
    }
}

/// A tick on the CPU running sporadic thread `t`: it drops to its low
/// priority if it used up its budget.
pub fn budget_check(t: &Thread) {
    let ran = t.cpu_time();
    let drop = {
        let mut ss = t.ss.lock();
        match ss.as_mut() {
            Some(s) if s.normal_prio == 0 && s.deadline != 0 && ran >= s.deadline => {
                replenish(t, s, ran, clock::now_ns())
            }
            _ => None,
        }
    };
    if let Some(low) = drop {
        sched::set_base_priority(t, low);
    }
}

/// Sporadic thread `t` blocks, with `ran` ns of run time.
pub fn blocked(t: &Thread, ran: u64) {
    let drop = {
        let mut ss = t.ss.lock();
        match ss.as_mut() {
            Some(s) if s.normal_prio == 0 => replenish(t, s, ran, clock::now_ns()),
            _ => None,
        }
    };
    if let Some(low) = drop {
        sched::set_base_priority(t, low);
    }
}

/// Sporadic thread `t` unblocks at monotonic time `now`: a new activation,
/// at its normal priority.
pub fn woken(t: &Thread, now: u64) {
    if let Some(s) = t.ss.lock().as_mut()
        && s.normal_prio == 0
    {
        s.activation = now;
    }
}

/// One more chunk of `t`'s run time ends at `ran` (procnto's
/// `replenish_sched`): the low priority to drop to, if it does.
fn replenish(t: &Thread, s: &mut SsInfo, ran: u64, now: u64) -> Option<u8> {
    if s.deadline == 0 {
        s.act_runtime = ran;
        s.deadline = ran.saturating_add(s.init_budget);
        s.activation = now;
    }
    let exhausted = s.deadline < ran || s.repl_count + 1 >= s.max_repl;
    if !exhausted {
        let used = ran.saturating_sub(s.act_runtime);
        s.act_runtime = ran;
        if s.repl_count != 0 {
            s.repl_count += 1;
            s.amount = s.amount.saturating_add(used);
            return None;
        }
        s.amount = used;
    } else if s.repl_count != 0 {
        s.repl_count += 1;
        return Some(drop_to_low(t, s));
    }
    // The first chunk: its replenishment is due a period after the run
    // began.
    let at = s.activation.saturating_add(s.repl_period);
    if let Some(old) = s.repl_at.replace(at) {
        QUEUE.lock().remove(&(old, t.tid));
    }
    QUEUE.lock().insert((at, t.tid), t.arc());
    clock::deadline_armed(0, at);
    s.repl_count = 1;
    exhausted.then(|| drop_to_low(t, s))
}

fn drop_to_low(t: &Thread, s: &mut SsInfo) -> u8 {
    s.deadline = u64::MAX;
    s.normal_prio = t.base_priority();
    s.low_priority
}

/// The first replenishment due (monotonic ns).
pub fn next_deadline() -> Option<u64> {
    QUEUE.lock().keys().next().map(|&(at, _)| at)
}

/// Do the replenishments due (on CPU 0's tick).
pub fn expire() {
    let now = clock::now_ns();
    loop {
        let t = {
            let mut q = QUEUE.lock();
            let Some(&key) = q.keys().next().filter(|k| k.0 <= now) else {
                return;
            };
            q.remove(&key).expect("present")
        };
        let raise = {
            let mut ss = t.ss.lock();
            let Some(s) = ss.as_mut() else { continue };
            s.repl_at = None;
            s.repl_count = 0;
            if s.normal_prio != 0 {
                // A fresh budget at its normal priority.
                let prio = core::mem::take(&mut s.normal_prio);
                s.deadline = 0;
                Some(prio)
            } else {
                s.deadline = s.deadline.saturating_add(s.amount);
                None
            }
        };
        if let Some(prio) = raise {
            sched::set_base_priority(&t, prio);
        }
        // Running: its CPU programs the budget's new end.
        sched::reassess(&t);
    }
}

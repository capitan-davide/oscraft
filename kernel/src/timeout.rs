//! Timeouts: threads that sleep until a point in time (QNX's NANOSLEEP
//! state), and blocking kernel calls with a deadline (`TimerTimeout`).
//! Each CPU has its own queue, ordered by deadline (as procnto's per-CPU
//! timer queues): a timeout belongs to the CPU it was armed on, and the
//! tick of the CPU that looks
//! after that CPU's timers expires it (`clock`), so the resolution is one
//! tick, and it waits while that CPU's clock is held back. An idle CPU
//! takes only the tick of its first deadline (`clock::stop_tick`). An
//! expired sleeper is woken; any other blocked thread is unblocked with
//! ETIMEDOUT (`ker::unblock`).

use alloc::collections::BTreeMap;

use crate::clock;
use crate::ker::errno::ETIMEDOUT;
use crate::percpu::MAX_CPUS;
use crate::sched::{self, State, ThreadRef};
use crate::sync::IrqSpinLock;

/// A CPU's timeouts: (deadline in ns of the monotonic clock, tid) ->
/// waiting thread. One queue per CPU, so that a tick looks only at the
/// queues of its family, each from its first entry (review CORE-27: one
/// shared queue was scanned past other CPUs' entries, with interrupts
/// masked).
type Queue = BTreeMap<(u64, u32), ThreadRef>;
static QUEUES: [IrqSpinLock<Queue>; MAX_CPUS] = [const { IrqSpinLock::new(BTreeMap::new()) }; MAX_CPUS];

/// A queued timeout, as `disarm` takes it back: its deadline and the CPU
/// whose queue holds it.
#[derive(Debug, Clone, Copy)]
pub struct Armed {
    deadline: u64,
    cpu: u32,
}

/// Queue the calling thread's timeout at `deadline` in `q`, CPU `cpu`'s
/// queue (the caller holds its lock).
fn queue(q: &mut Queue, cpu: u32, deadline: u64) -> Armed {
    let me = sched::current();
    q.insert((deadline, me.tid), me.arc());
    Armed { deadline, cpu }
}

/// The calling CPU, for its queue: the timeout is armed on it.
fn this_cpu() -> u32 {
    crate::percpu::this().cpu
}

/// Block the calling thread until the monotonic clock reaches `deadline`
/// (ns), or until something else wakes it.
pub fn sleep_until(deadline: u64) {
    if clock::now_ns() >= deadline {
        return;
    }
    // The queue's lock is held from the state change to the insertion:
    // the tick can't find this thread asleep and not queued.
    let cpu = this_cpu();
    let a = {
        let mut q = QUEUES[cpu as usize].lock();
        sched::prepare_block(State::Nanosleep);
        queue(&mut q, cpu, deadline)
    };
    clock::deadline_armed(a.cpu, deadline);
    sched::block();
    disarm(Some(a));
}

/// The `_NTO_TIMEOUT_*` bit of a blocking state (`1 << STATE_*`,
/// `<sys/states.h>`).
fn qnx_state_bit(state: State) -> u32 {
    if state.is_blocked() {
        1 << state.qnx()
    } else {
        0
    }
}

/// The calling thread is about to block in `state` (already marked so,
/// with the lock of what it waits on held): if a `TimerTimeout` armed for
/// that state is pending, start it. Returns what `disarm` takes.
///
/// The timeout is the kernel call's, not its first block's: a call woken
/// with nothing to return blocks again with the same deadline (it is
/// forgotten as the thread's next kernel call starts). A deadline already
/// passed (`TimerTimeout` 0, as libc's `poll()` drains its signals) ends a
/// signal wait or a sleep at once, as on procnto, not at the next tick.
pub fn arm(state: State) -> Option<Armed> {
    arm_for(state, qnx_state_bit(state))
}

/// `arm` for a `MsgSend()` that blocks in SEND: procnto starts the
/// timeout if the mask has `_NTO_TIMEOUT_SEND` or `_NTO_TIMEOUT_REPLY`,
/// and once started it runs on through the server's receive, in REPLY
/// (measured, IPC-3: SEND alone, REPLY alone and both all end a send
/// that queued with ETIMEDOUT at the deadline, whichever state it is in
/// then; SEND alone does nothing for a send the server was waiting for,
/// which blocks in REPLY from the start).
pub fn arm_send() -> Option<Armed> {
    arm_for(State::Send, qnx_state_bit(State::Send) | qnx_state_bit(State::Reply))
}

fn arm_for(state: State, mask: u32) -> Option<Armed> {
    let me = sched::current();
    if !me.timeout_armed.load(core::sync::atomic::Ordering::Relaxed) {
        return None;
    }
    let (deadline, states) = (*me.armed_timeout.lock())?;
    if states & mask == 0 {
        return None;
    }
    if deadline <= clock::now_ns()
        && matches!(state, State::Sigwaitinfo | State::Sigsuspend | State::Join | State::Intr)
    {
        sched::wake_with(me, ETIMEDOUT);
        return None;
    }
    let cpu = this_cpu();
    let a = queue(&mut QUEUES[cpu as usize].lock(), cpu, deadline);
    clock::deadline_armed(a.cpu, deadline);
    Some(a)
}

/// Has the calling thread a `TimerTimeout` for `state` whose deadline has
/// passed (libc's trylocks arm one)?
pub fn immediate(state: State) -> bool {
    let me = sched::current();
    if !me.timeout_armed.load(core::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    let Some((deadline, states)) = *me.armed_timeout.lock() else {
        return false;
    };
    states & qnx_state_bit(state) != 0 && deadline <= clock::now_ns()
}

/// The calling thread has stopped waiting: forget its timeout.
pub fn disarm(armed: Option<Armed>) {
    if let Some(a) = armed {
        QUEUES[a.cpu as usize].lock().remove(&(a.deadline, sched::current().tid));
    }
}

/// The CPUs of `family` (bit n: CPU n).
fn cpus(family: u64) -> impl Iterator<Item = usize> {
    (0..MAX_CPUS).filter(move |&c| family & 1 << c != 0)
}

/// The first deadline (ns) of the timeouts of the CPUs of `family`.
pub fn next_deadline(family: u64) -> Option<u64> {
    cpus(family).filter_map(|c| QUEUES[c].lock().first_key_value().map(|(&(d, _), _)| d)).min()
}

/// Expire the timeouts of the CPUs of `family` whose deadline has passed
/// (from the clock tick of the CPU that looks after them).
pub fn expire(family: u64) {
    let now = clock::now_ns();
    for c in cpus(family) {
        // One due entry at a time, the lock let go before the wake.
        loop {
            let t = {
                let mut q = QUEUES[c].lock();
                match q.first_entry() {
                    Some(e) if e.key().0 <= now => e.remove(),
                    _ => break,
                }
            };
            if t.state() == State::Nanosleep {
                sched::wake(&t);
            } else {
                crate::ker::unblock(&t, ETIMEDOUT);
            }
        }
    }
}

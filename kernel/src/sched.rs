//! Threads and the scheduler.
//!
//! QNX semantics: 256 priority levels (0 is idle), FIFO and round-robin
//! policies (`SCHED_OTHER` is round robin too), strict priority. A thread
//! preempted by a higher-priority one goes back to the head of its
//! priority's queue; one that yields or uses up its round-robin timeslice
//! goes to the tail.
//!
//! The timeslice is procnto's (`nto_sched_thread_deadline_handle`): 4
//! ticks less a quarter tick (3.75 ms) of the thread's own run time,
//! looked at on the ticks of the CPU that runs it. The slice's end is a
//! point in the thread's run time, set at the first tick it runs and moved
//! on by a slice each time it is reached (and the thread gives way): a
//! thread keeps what is left of its slice when it is preempted or blocks.
//! Two such threads on one CPU take turns of 4 ticks (as measured on
//! procnto: 4.000 ms). A sporadic thread's budget is looked at on the ticks
//! too.
//!
//! One ready queue for all CPUs, under an interrupt-masking lock. A thread
//! is running on one CPU (`on_cpu`), in the ready queue, blocked or dead.
//! The context switch clears `on_cpu` of the outgoing thread only once its
//! registers are saved, and a CPU about to resume a thread waits for that,
//! so a thread requeued by one CPU cannot be resumed by another while its
//! stack is still in use.
//!
//! Blocking: a thread sets a blocked state while holding the lock of the
//! object it waits on, releases that lock and calls `block`. A waker,
//! holding the same lock, moves it from blocked to ready (`wake`). If the
//! wake comes before the thread has switched away, `block` finds it ready
//! (and queued) and simply keeps running, so no wake-up is lost.
//!
//! Each CPU's boot context becomes its idle thread, but only enters the
//! idle loop when its boot work is done; until then it must not be
//! preempted (it is never queued, so it would not resume until the CPU had
//! nothing else to run).
//!
//! CPU time: a thread adds up the time from each switch to it to the next
//! switch away (`cpu_time()`), so an idle thread's time is its CPU's idle
//! time. Time in the kernel counts as the thread's.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, AtomicU64, Ordering};

use crate::arch::context::{self, Context};
use crate::arch::mmu::Prot;
use crate::arch::{self, irq_restore, irq_save};
use crate::percpu;
use crate::sync::IrqSpinLock;
use crate::vm::kmem;
use crate::vm::pmem::PAGE_SIZE;

pub const PRIO_IDLE: u8 = 0;
/// Round-robin timeslice in nanoseconds of run time: as procnto's
/// `rr_timeslice`, `rr_interval_mul` (4) ticks less a quarter tick.
pub fn rr_slice_ns() -> u64 {
    let tick = crate::clock::tick_ns();
    4 * tick - tick / 4
}
/// Kernel stack of a thread; unoptimized builds build large values (a
/// `Process`) on the stack, and need more.
const STACK_PAGES: u64 = if cfg!(debug_assertions) { 16 } else { 4 };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Policy {
    Fifo = 0,
    RoundRobin = 1,
    /// `SCHED_OTHER`: scheduled as round robin, but reported as itself.
    Other = 2,
    /// `SCHED_SPORADIC`: FIFO within a budget of run time (`sporadic`).
    Sporadic = 3,
    /// `SCHED_HIGH_PRIORITY_IST`: FIFO, for interrupt threads, up to 254.
    HighPriorityIst = 4,
    /// `SCHED_OFFLINING`: FIFO, up to 254, for the threads that take
    /// CPUs offline (`ker::offline`).
    Offlining = 5,
}

impl Policy {
    /// Round robin (a timeslice)?
    pub fn sliced(self) -> bool {
        matches!(self, Policy::RoundRobin | Policy::Other)
    }
}

/// Thread states (the blocked ones are named as in QNX).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Ready = 0,
    Running = 1,
    Dead = 2,
    /// SEND-blocked: message sent, not yet received.
    Send = 3,
    /// RECEIVE-blocked: waiting for a message or pulse.
    Receive = 4,
    /// REPLY-blocked: message received, not yet replied to.
    Reply = 5,
    /// Sleeping until a deadline, see `timeout`.
    Nanosleep = 6,
    /// Waiting for a signal (`SignalSuspend`), see `ker::signal`.
    Sigsuspend = 7,
    /// Waiting for a mutex, see `ker::sync`.
    Mutex = 8,
    /// Waiting on a condition variable, see `ker::sync`.
    Condvar = 9,
    /// Waiting for a signal of a set (`SignalWaitinfo`).
    Sigwaitinfo = 10,
    /// Waiting for an interrupt (`InterruptWait`), see `ker::interrupt`.
    Intr = 11,
    /// Waiting for a thread to end (`ThreadJoin`), see `ker::thread`.
    Join = 12,
    /// Stopped by job control or a debugger, see `ker::debug`.
    Stopped = 13,
    /// Waiting on a semaphore (`SyncSemWait`), see `ker::sync`.
    Sem = 14,
    /// Waiting for a full trace buffer (`_NTO_TRACE_WAITBUFFER`), see
    /// `ker::trace`.
    TraceBuffer = 15,
    /// Waiting at a barrier (`SyncBarrierWait`), see `ker::sync`.
    Barrier = 16,
    /// Waiting for room in a message queue, see `ker::mqueue`.
    MqSend = 17,
    /// Waiting for a message in a message queue.
    MqReceive = 18,
    /// Waiting to read from or write to a kernel pipe, see `ker::kpipe`.
    Pipe = 19,
}

impl State {
    fn from_u8(v: u8) -> Self {
        match v {
            0 => State::Ready,
            1 => State::Running,
            3 => State::Send,
            4 => State::Receive,
            5 => State::Reply,
            6 => State::Nanosleep,
            7 => State::Sigsuspend,
            8 => State::Mutex,
            9 => State::Condvar,
            10 => State::Sigwaitinfo,
            11 => State::Intr,
            12 => State::Join,
            13 => State::Stopped,
            14 => State::Sem,
            15 => State::TraceBuffer,
            16 => State::Barrier,
            17 => State::MqSend,
            18 => State::MqReceive,
            19 => State::Pipe,
            _ => State::Dead,
        }
    }

    /// The QNX state number (`STATE_*`, `<sys/states.h>`).
    pub fn qnx(self) -> u8 {
        match self {
            State::Dead => 0,
            State::Running => 1,
            State::Ready => 2,
            State::Send => 4,
            State::Receive => 5,
            State::Reply => 6,
            State::Sigsuspend => 10,
            State::Sigwaitinfo => 11,
            State::Nanosleep => 12,
            State::Mutex => 13,
            State::Condvar => 14,
            State::Intr => 16,
            State::Join => 15,
            State::Stopped => 3,
            State::Sem => 17,
            State::TraceBuffer => 27,
            State::Barrier => 21,
            State::MqSend => 7,
            State::MqReceive => 8,
            State::Pipe => 22,
        }
    }

    pub fn is_blocked(self) -> bool {
        matches!(
            self,
            State::Send
                | State::Receive
                | State::Reply
                | State::Nanosleep
                | State::Sigsuspend
                | State::Mutex
                | State::Condvar
                | State::Sigwaitinfo
                | State::Intr
                | State::Join
                | State::Stopped
                | State::Sem
                | State::TraceBuffer
                | State::Barrier
                | State::MqSend
                | State::MqReceive
                | State::Pipe
        )
    }
}

pub struct Thread {
    pub tid: u32,
    /// QNX thread ID: per process, the first user thread is 1 (kernel
    /// threads: their global tid).
    pub ptid: u32,
    pub name: &'static str,
    /// Effective priority (may be raised or lowered by message passing).
    prio: AtomicU8,
    /// Priority set by the thread itself (or its creator).
    base_prio: AtomicU8,
    /// `Policy` as its number (0..=5); see `policy()`.
    policy: AtomicU8,
    /// Owning process (procnto for kernel threads).
    pub process: crate::ker::process::ProcessRef,
    state: AtomicU8,
    on_cpu: AtomicU8,
    /// Its run time (`cpu_time()`) at which its round-robin timeslice
    /// ends.
    slice_end: AtomicU64,
    /// It reached its timeslice's end: it gives way to an equal priority.
    slice_out: AtomicBool,
    /// Its sporadic scheduling state (`Policy::Sporadic`).
    pub ss: IrqSpinLock<Option<crate::sporadic::SsInfo>>,
    /// Bitmask of the CPUs this thread has run on (statistics).
    pub cpus_seen: AtomicU64,
    /// Nanoseconds it has run, up to its last switch away from a CPU; see
    /// `cpu_time()`.
    cpu_ns: AtomicU64,
    /// When it started running on the CPU it is on (`clock::now_ns()`);
    /// `NOT_RUNNING` while it is on none.
    run_since: AtomicU64,
    /// The CPU it last ran on.
    last_cpu: AtomicU8,
    /// When it was created (`clock::now_ns()`).
    pub start_time: u64,
    /// Message-passing state, see `ker::msg`.
    pub ipc: crate::ker::msg::ThreadIpc,
    /// Blocked signals (bit n-1 for signal n), see `ker::signal`.
    pub sigmask: crate::ker::signal::AtomicSigSet,
    /// The CPUs it may run on (`_NTO_TCTL_RUNMASK`), bit n for CPU n.
    pub runmask: AtomicU64,
    /// The first thread of a forked child (or of a boot script command
    /// after `reopen`): the `_IO_DUP`s to send before it first runs, (coid,
    /// parent pid, server chid, parent's scoid, parent's coid); see
    /// `ker::fork_startup`.
    pub pending_dups: crate::sync::IrqSpinLock<Option<Vec<[i32; 5]>>>,
    /// Close-on-exec connections to other servers an exec left for the
    /// new program's first thread to close with `_IO_CLOSE` before it
    /// runs (`ker::fork_startup`; review PROCMGR-8).
    pub pending_closes: crate::sync::IrqSpinLock<Option<Vec<i32>>>,
    /// The first thread of a spawned child: the files it opens before it
    /// first runs (open file actions naming other servers' files).
    pub pending_opens: crate::sync::IrqSpinLock<Option<Vec<crate::ker::StartupOpen>>>,
    /// Why a wait other than message passing ended early (ETIMEDOUT,
    /// EINTR), for the call to return; 0 if it did not.
    pub wake_error: AtomicU32,
    /// The connection (its address) it waits in a kernel pipe through,
    /// for `_NTO_PIPE_CTL_UNBLOCK`.
    pub pipe_conn: core::sync::atomic::AtomicUsize,
    /// A REPLY-blocked thread's server has been asked to unblock it
    /// (`_PULSE_CODE_UNBLOCK`).
    pub unblock_requested: AtomicBool,
    /// In a kernel call whose caller had interrupts enabled (from EL0, or
    /// a kernel thread holding no spinlock): a long operation in it may
    /// let a higher-priority thread have the CPU (`preemption_point`).
    pub kcall_preemptible: AtomicBool,
    /// `TimerTimeout` armed for the next blocking call: (monotonic deadline
    /// in ns, `_NTO_TIMEOUT_*` state bits).
    pub armed_timeout: crate::sync::IrqSpinLock<Option<(u64, u32)>>,
    /// A `TimerTimeout` may be armed (`armed_timeout` set): the next kernel
    /// call clears it.
    pub timeout_armed: AtomicBool,
    /// The signals a `SignalWaitinfo` waits for.
    pub sigwait_set: crate::ker::signal::AtomicSigSet,
    /// A `SIGEV_INTR` event has fired since the last `InterruptWait`.
    pub intr_pending: AtomicBool,
    /// The signal mask to restore once a signal has been handled, while
    /// `SignalSuspend` has replaced it (`SUSPEND_NONE`: none).
    pub suspend_mask: crate::ker::signal::AtomicSigSet,
    /// Its alternate signal stack (`sigaltstack()`): base and size, size
    /// 0 when it has none.
    pub altstack: crate::sync::IrqSpinLock<(u64, u64)>,
    /// A user stack the kernel mapped for it (a `SIGEV_THREAD` thread's):
    /// unmapped when it ends.
    pub owned_stack: crate::sync::IrqSpinLock<Option<(u64, u64)>>,
    /// End the thread when its current kernel call returns (the thread
    /// that called exec(), replaced by the new program's).
    pub exit_on_return: AtomicBool,
    /// Added to its CPU-time clock's reading since it was set, wrapping.
    pub cpu_clock_offset: AtomicU64,
    /// Held (`_NTO_TCTL_ONE_THREAD_HOLD`, `_THREADS_HOLD`): it parks, as
    /// stopped, before it returns to EL0, until continued.
    pub held: AtomicBool,
    /// Misaligned accesses fault (`_NTO_TCTL_ALIGN_FAULT` > 0): its EL0
    /// code runs with `SCTLR_EL1.A` set.
    pub align_fault: AtomicBool,
    /// The event to deliver when it ends (`_NTO_TCTL_ADD_EXIT_EVENT`), and
    /// the process that added it.
    pub exit_event: crate::sync::IrqSpinLock<Option<(i32, crate::ker::event::SigEvent)>>,
    /// Its thread pointer as it started (its TLS), for debuggers.
    pub tls: AtomicU64,
    /// Cancelled (`ThreadCancel()`): where it goes at its next cancellation
    /// point (libc's stub); 0 if not.
    pub cancel_stub: AtomicU64,
    /// Detached (`ThreadDetach()`): nobody can join it.
    pub detached: AtomicBool,
    /// Joined already: nobody else can.
    pub joined: AtomicBool,
    /// Its exit status (`ThreadDestroy()`'s), for `ThreadJoin()`.
    pub exit_status: AtomicU64,
    /// The thread waiting in `ThreadJoin()` for it to end; see `JOIN_LOCK`.
    pub joiner: crate::sync::IrqSpinLock<Option<ThreadRef>>,
    /// The name it was given (`_NTO_TCTL_NAME`); empty: none.
    pub qnx_name: crate::sync::IrqSpinLock<alloc::string::String>,
    /// Top of the kernel stack of a user thread (where its user state is
    /// saved on kernel entry); 0 for kernel threads.
    kstack_top: u64,
    /// The thread's kernel stack (first page, pages), freed once the
    /// thread is dead and off its CPU; (0, 0) for boot contexts.
    stack: (u64, u64),
    /// FP/SIMD registers of a user thread (kernel threads have none).
    fp: Option<Box<UnsafeCell<crate::arch::context::FpState>>>,
    /// Its FP/SIMD state is in the registers of the CPU it runs on (loaded
    /// at its first FP instruction since it was switched in, see
    /// `fp_make_live`), not only in `fp`.
    fp_live: AtomicBool,
    context: UnsafeCell<Context>,
    /// Itself, for `arc()`.
    this: Weak<Thread>,
}

// SAFETY: `context` is only accessed by the CPU that owns the thread (it is
// running there, or that CPU is switching to or from it under the on_cpu
// protocol); everything else is atomic or has its own locking rules.
unsafe impl Sync for Thread {}

impl Thread {
    /// Its kernel stack: (first page, pages); (0, 0) for a CPU's boot
    /// context.
    pub fn kernel_stack(&self) -> (u64, u64) {
        self.stack
    }

    /// May it run on CPU `cpu`: in its runmask, and only a
    /// `SCHED_OFFLINING` thread on a CPU taken offline (`ker::offline`)?
    pub fn may_run_on(&self, cpu: u32) -> bool {
        self.runmask.load(Ordering::Relaxed) & (1 << cpu) != 0
            && (self.policy() == Policy::Offlining
                || !percpu::get(cpu).is_some_and(|c| c.offline.load(Ordering::Acquire)))
    }

    /// A counted reference to this thread, to keep (in a queue...).
    pub fn arc(&self) -> ThreadRef {
        self.this.upgrade().expect("thread being freed")
    }

    pub fn state(&self) -> State {
        State::from_u8(self.state.load(Ordering::Acquire))
    }

    /// Nanoseconds this thread has run (its CPU-time clock).
    pub fn cpu_time(&self) -> u64 {
        loop {
            // A switch away in between adds the run to `cpu_ns`: retry.
            let since = self.run_since.load(Ordering::Acquire);
            let ns = self.cpu_ns.load(Ordering::Acquire);
            if self.run_since.load(Ordering::Acquire) != since {
                continue;
            }
            return match since {
                NOT_RUNNING => ns,
                _ => ns + crate::clock::now_ns().saturating_sub(since),
            };
        }
    }

    /// The CPU this thread last ran on.
    pub fn last_cpu(&self) -> u8 {
        self.last_cpu.load(Ordering::Relaxed)
    }

    pub fn priority(&self) -> u8 {
        self.prio.load(Ordering::Relaxed)
    }

    pub fn base_priority(&self) -> u8 {
        self.base_prio.load(Ordering::Relaxed)
    }

    pub fn policy(&self) -> Policy {
        match self.policy.load(Ordering::Relaxed) {
            0 => Policy::Fifo,
            1 => Policy::RoundRobin,
            2 => Policy::Other,
            3 => Policy::Sporadic,
            4 => Policy::HighPriorityIst,
            _ => Policy::Offlining,
        }
    }

    pub fn set_policy(&self, policy: Policy) {
        self.policy.store(policy as u8, Ordering::Relaxed);
    }
}

/// A counted reference to a thread. The thread table holds one until the
/// thread is dead and off its CPU (see `reap`); the thread is freed with
/// the last one. Code running on behalf of a thread uses `current()`, a
/// plain reference, as its thread can't be freed while that code runs.
pub type ThreadRef = Arc<Thread>;

struct ReadyQueue {
    bitmap: [u64; 4],
    queues: [VecDeque<ThreadRef>; 256],
}

impl ReadyQueue {
    fn push(&mut self, t: ThreadRef, front: bool) {
        let p = t.priority() as usize;
        if front {
            self.queues[p].push_front(t);
        } else {
            self.queues[p].push_back(t);
        }
        self.bitmap[p / 64] |= 1 << (p % 64);
    }

    /// The highest priority with a thread that may run on `cpu`.
    fn highest_for(&self, cpu: u32) -> Option<u8> {
        // The priorities with ready threads, highest first.
        for w in (0..4).rev() {
            let mut bits = self.bitmap[w];
            while bits != 0 {
                let b = 63 - bits.leading_zeros() as usize;
                bits &= !(1 << b);
                let p = w * 64 + b;
                if self.queues[p].iter().any(|t| t.may_run_on(cpu)) {
                    return Some(p as u8);
                }
            }
        }
        None
    }

    /// The first thread of priority `p` that may run on `cpu`.
    fn pop_for(&mut self, p: u8, cpu: u32) -> Option<ThreadRef> {
        let q = &mut self.queues[p as usize];
        let t = q
            .iter()
            .position(|t| t.may_run_on(cpu))
            .and_then(|i| q.remove(i));
        self.update_bit(p);
        t
    }

    /// Remove `t` from the list of its current priority.
    fn remove(&mut self, t: &Thread) -> bool {
        let p = t.priority();
        let q = &mut self.queues[p as usize];
        let before = q.len();
        q.retain(|x| !core::ptr::eq(&**x, t));
        let removed = q.len() != before;
        self.update_bit(p);
        removed
    }

    fn update_bit(&mut self, p: u8) {
        if self.queues[p as usize].is_empty() {
            self.bitmap[p as usize / 64] &= !(1 << (p % 64));
        }
    }
}

static READY: IrqSpinLock<ReadyQueue> = IrqSpinLock::new(ReadyQueue {
    bitmap: [0; 4],
    queues: [const { VecDeque::new() }; 256],
});
static NEXT_TID: AtomicU32 = AtomicU32::new(1);
/// All threads by tid, for looking them up from receive IDs; a thread
/// leaves once it is dead and off its CPU (`reap`).
static THREADS: IrqSpinLock<BTreeMap<u32, ThreadRef>> = IrqSpinLock::new(BTreeMap::new());

/// Per-CPU scheduler state, part of `percpu::PerCpu`.
#[derive(Debug)]
pub struct CpuSched {
    current: AtomicPtr<Thread>,
    idle: AtomicPtr<Thread>,
    need_resched: AtomicBool,
    /// Set when the boot context enters the idle loop.
    preemptible: AtomicBool,
}

impl CpuSched {
    pub const fn new() -> Self {
        Self {
            current: AtomicPtr::new(core::ptr::null_mut()),
            idle: AtomicPtr::new(core::ptr::null_mut()),
            need_resched: AtomicBool::new(false),
            preemptible: AtomicBool::new(false),
        }
    }
}

fn register(t: Thread) -> ThreadRef {
    let t = Arc::new_cyclic(|this| Thread {
        this: this.clone(),
        ..t
    });
    THREADS.lock().insert(t.tid, t.clone());
    t
}

fn new_thread_struct(
    name: &'static str,
    priority: u8,
    policy: Policy,
    context: Context,
    process: &crate::ker::process::Process,
) -> Thread {
    process.thread_started();
    let process = process.arc();
    let tid = NEXT_TID.fetch_add(1, Ordering::Relaxed);
    Thread {
        tid,
        ptid: tid,
        name,
        prio: AtomicU8::new(priority),
        base_prio: AtomicU8::new(priority),
        policy: AtomicU8::new(policy as u8),
        process,
        state: AtomicU8::new(State::Ready as u8),
        on_cpu: AtomicU8::new(0),
        // A new thread's first slice ends at 3.75 ms of its run time (as
        // procnto sets it at creation).
        slice_end: AtomicU64::new(rr_slice_ns()),
        slice_out: AtomicBool::new(false),
        ss: IrqSpinLock::new(None),
        cpus_seen: AtomicU64::new(0),
        cpu_ns: AtomicU64::new(0),
        run_since: AtomicU64::new(NOT_RUNNING),
        last_cpu: AtomicU8::new(0),
        start_time: crate::clock::now_ns(),
        ipc: crate::ker::msg::ThreadIpc::new(),
        sigmask: crate::ker::signal::AtomicSigSet::new(crate::ker::signal::SPECIAL),
        runmask: AtomicU64::new(u64::MAX),
        exit_on_return: AtomicBool::new(false),
        cpu_clock_offset: AtomicU64::new(0),
        held: AtomicBool::new(false),
        align_fault: AtomicBool::new(false),
        exit_event: crate::sync::IrqSpinLock::new(None),
        tls: AtomicU64::new(0),
        cancel_stub: AtomicU64::new(0),
        detached: AtomicBool::new(false),
        joined: AtomicBool::new(false),
        exit_status: AtomicU64::new(0),
        joiner: crate::sync::IrqSpinLock::new(None),
        qnx_name: crate::sync::IrqSpinLock::new(alloc::string::String::new()),
        suspend_mask: crate::ker::signal::AtomicSigSet::new(SUSPEND_NONE),
        altstack: crate::sync::IrqSpinLock::new((0, 0)),
        owned_stack: crate::sync::IrqSpinLock::new(None),
        sigwait_set: crate::ker::signal::AtomicSigSet::new(0),
        intr_pending: AtomicBool::new(false),
        armed_timeout: crate::sync::IrqSpinLock::new(None),
        timeout_armed: AtomicBool::new(false),
        wake_error: AtomicU32::new(0),
        pipe_conn: core::sync::atomic::AtomicUsize::new(0),
        unblock_requested: AtomicBool::new(false),
        kcall_preemptible: AtomicBool::new(false),
        pending_dups: crate::sync::IrqSpinLock::new(None),
        pending_closes: crate::sync::IrqSpinLock::new(None),
        pending_opens: crate::sync::IrqSpinLock::new(None),
        kstack_top: 0,
        stack: (0, 0),
        fp: None,
        fp_live: AtomicBool::new(false),
        context: UnsafeCell::new(context),
        this: Weak::new(),
    }
}

/// `Thread::run_since` of a thread on no CPU.
const NOT_RUNNING: u64 = u64::MAX;

/// `Thread::suspend_mask` when no mask is saved.
pub const SUSPEND_NONE: crate::ker::signal::SigSet = u128::MAX;

/// The live threads of a process.
pub fn threads_of(p: &crate::ker::process::Process) -> Vec<ThreadRef> {
    THREADS
        .lock()
        .values()
        .filter(|t| core::ptr::eq(&*t.process, p) && t.state() != State::Dead)
        .cloned()
        .collect()
}

/// Wait until no thread of process `p` is on a CPU (they have all ended:
/// its address space can go).
pub fn wait_off_cpu(p: &crate::ker::process::Process) {
    let threads: Vec<ThreadRef> = THREADS
        .lock()
        .values()
        .filter(|t| core::ptr::eq(&*t.process, p))
        .cloned()
        .collect();
    for t in threads {
        while t.on_cpu.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
    }
}

/// Wait until no thread of process `p` but `keep` is on a CPU.
pub fn wait_off_cpu_but(p: &crate::ker::process::Process, keep: &Thread) {
    let threads: Vec<ThreadRef> = THREADS
        .lock()
        .values()
        .filter(|t| core::ptr::eq(&*t.process, p) && !core::ptr::eq(&***t, keep))
        .cloned()
        .collect();
    for t in threads {
        while t.on_cpu.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
    }
}

/// Orders a thread's end with `ThreadJoin()`'s wait for it.
pub static JOIN_LOCK: IrqSpinLock<()> = IrqSpinLock::new(());

/// The thread of process `p` with QNX thread ID `ptid`: live, or dead and
/// not yet joined (a zombie for `ThreadJoin()`).
pub fn any_thread_of(p: &crate::ker::process::Process, ptid: u32) -> Option<ThreadRef> {
    // A dead thread that was joined or detached has given its ID back,
    // though it may wait in the table to be reaped: a new thread may have
    // the ID now, and is the one meant.
    THREADS
        .lock()
        .values()
        .find(|t| {
            core::ptr::eq(&*t.process, p)
                && t.ptid == ptid
                && !(t.state() == State::Dead
                    && (t.joined.load(Ordering::Acquire) || t.detached.load(Ordering::Acquire)))
        })
        .cloned()
}

/// The live thread of process `p` with QNX thread ID `ptid`.
pub fn thread_of(p: &crate::ker::process::Process, ptid: u32) -> Option<ThreadRef> {
    // Without a list of them all: also called from interrupt handlers
    // (`ker::event::deliver_from`).
    THREADS
        .lock()
        .values()
        .find(|t| core::ptr::eq(&*t.process, p) && t.ptid == ptid && t.state() != State::Dead)
        .cloned()
}

/// The thread with this tid, if it exists and is not dead.
pub fn thread(tid: u32) -> Option<ThreadRef> {
    THREADS
        .lock()
        .get(&tid)
        .filter(|t| t.state() != State::Dead)
        .cloned()
}

/// Turn the calling CPU's boot context into its idle thread.
pub fn init_cpu() {
    let me = percpu::this();
    let mut idle = new_thread_struct(
        "idle",
        PRIO_IDLE,
        Policy::Fifo,
        Context::default(),
        &crate::ker::process::KERNEL,
    );
    // As in QNX, process 1's threads 1..=num_cpu are the CPUs' idle threads
    // (top reads each CPU's idle time from them). They are created first,
    // so these IDs are free.
    idle.ptid = me.cpu + 1;
    let idle = register(idle);
    idle.state.store(State::Running as u8, Ordering::Relaxed);
    idle.on_cpu.store(1, Ordering::Relaxed);
    idle.run_since
        .store(crate::clock::now_ns(), Ordering::Relaxed);
    idle.last_cpu.store(me.cpu as u8, Ordering::Relaxed);
    let p = Arc::as_ptr(&idle) as *mut Thread;
    me.sched.idle.store(p, Ordering::Relaxed);
    me.sched.current.store(p, Ordering::Release);
}

/// The calling thread. It lives at least as long as the code that runs
/// on its behalf; to keep it beyond, take `arc()`.
pub fn current() -> &'static Thread {
    // SAFETY: set by init_cpu before any scheduling on this CPU; the
    // thread table keeps the running thread (`ThreadRef`).
    unsafe { &*percpu::this().sched.current.load(Ordering::Acquire) }
}

/// Create a kernel thread running `entry(arg)` and make it ready.
pub fn spawn(
    name: &'static str,
    priority: u8,
    policy: Policy,
    entry: fn(usize),
    arg: usize,
) -> Option<ThreadRef> {
    assert!(priority > PRIO_IDLE, "priority 0 is reserved for idle");
    let stack = kmem::map_new_guarded(STACK_PAGES, Prot::Data)?;
    let ctx = Context::new_thread(
        stack + STACK_PAGES * PAGE_SIZE,
        entry as usize as u64,
        arg as u64,
    );
    let mut t = new_thread_struct(name, priority, policy, ctx, &crate::ker::process::KERNEL);
    t.stack = (stack, STACK_PAGES);
    let t = register(t);
    crate::ker::trace::thread_created(&t);
    crate::ker::trace::thread_state(&t, State::Ready);
    READY.lock().push(t.clone(), false);
    kick_for(&t, false);
    Some(t)
}

/// Create a thread of user process `process` that starts at EL0 in
/// `entry(arg)` with stack pointer `sp` and thread pointer `tp`, and make
/// it ready.
pub fn spawn_user(
    process: &crate::ker::process::Process,
    name: &'static str,
    priority: u8,
    entry: u64,
    sp: u64,
    arg: u64,
    tp: u64,
) -> Option<ThreadRef> {
    use crate::arch::context::FpState;
    use crate::arch::exception::TrapFrame;
    let frame = TrapFrame::new_user(entry, sp, arg);
    spawn_user_frame(process, name, priority, frame, tp, FpState::new(), 0, 1)
}

/// A user thread of `process` that starts in the user state `frame`, with
/// thread pointer `tp`, FP/SIMD registers `fp` and blocked signals
/// `sigmask` (for fork()), with QNX thread ID `ptid`.
#[allow(clippy::too_many_arguments)]
pub fn spawn_user_frame(
    process: &crate::ker::process::Process,
    name: &'static str,
    priority: u8,
    frame: crate::arch::exception::TrapFrame,
    tp: u64,
    fp: crate::arch::context::FpState,
    sigmask: crate::ker::signal::SigSet,
    ptid: u32,
) -> Option<ThreadRef> {
    spawn_user_frame_init(
        process,
        name,
        priority,
        frame,
        tp,
        fp,
        sigmask,
        ptid,
        |_| {},
    )
}

/// `spawn_user_frame`, with `init` run on the thread before it can run.
#[allow(clippy::too_many_arguments)]
pub fn spawn_user_frame_init(
    process: &crate::ker::process::Process,
    name: &'static str,
    priority: u8,
    frame: crate::arch::exception::TrapFrame,
    tp: u64,
    fp: crate::arch::context::FpState,
    sigmask: crate::ker::signal::SigSet,
    ptid: u32,
    init: impl FnOnce(&Thread),
) -> Option<ThreadRef> {
    use crate::arch::exception::TrapFrame;
    let stack = kmem::map_new_guarded(STACK_PAGES, Prot::Data)?;
    let top = stack + STACK_PAGES * PAGE_SIZE;
    let at = top - core::mem::size_of::<TrapFrame>() as u64;
    // SAFETY: the frame lies at the top of the new, unused kernel stack.
    unsafe { (at as *mut TrapFrame).write(frame) };
    let ctx = Context::new_user_thread(at, tp);
    let mut t = new_thread_struct(name, priority, Policy::RoundRobin, ctx, process);
    t.fp = Some(Box::new(UnsafeCell::new(fp)));
    t.kstack_top = top;
    t.stack = (stack, STACK_PAGES);
    t.ptid = ptid;
    t.sigmask = crate::ker::signal::AtomicSigSet::new(crate::ker::signal::valid_mask(sigmask));
    t.tls = AtomicU64::new(tp);
    init(&t);
    let t = register(t);
    crate::ker::trace::thread_created(&t);
    crate::ker::trace::thread_state(&t, State::Ready);
    READY.lock().push(t.clone(), false);
    // As a woken thread: an idle CPU of its runmask runs it, or this one
    // switches to it if it is higher.
    let higher = t.priority() > current().priority();
    if higher {
        percpu::this()
            .sched
            .need_resched
            .store(true, Ordering::Relaxed);
    }
    kick_for(&t, higher);
    Some(t)
}

impl Thread {
    /// Run `f` on the saved user state of this thread (to change its
    /// registers).
    ///
    /// # Safety
    ///
    /// The thread must stay blocked while this runs.
    pub unsafe fn with_user_frame(
        &self,
        f: impl FnOnce(&mut crate::arch::exception::TrapFrame),
    ) -> Option<()> {
        use crate::arch::exception::TrapFrame;
        self.fp.as_ref()?;
        if !self.state().is_blocked() {
            return None;
        }
        while self.on_cpu.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
        let at = self.kstack_top - core::mem::size_of::<TrapFrame>() as u64;
        // SAFETY: as in `user_state`; the thread doesn't run, so nothing
        // else uses its saved state.
        f(unsafe { &mut *(at as *mut TrapFrame) });
        Some(())
    }

    /// Run `f` on the saved FP state of this thread.
    ///
    /// # Safety
    ///
    /// The thread must stay blocked while this runs.
    pub unsafe fn with_user_fp(
        &self,
        f: impl FnOnce(&mut crate::arch::context::FpState),
    ) -> Option<()> {
        let fp = self.fp.as_ref()?;
        if !self.state().is_blocked() {
            return None;
        }
        while self.on_cpu.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
        // SAFETY: the thread doesn't run: its FP state is saved and ours.
        f(unsafe { &mut *fp.get() });
        Some(())
    }

    /// A copy of the user state of a user thread that is blocked in a
    /// kernel call (its trap frame, thread pointer and FP/SIMD registers).
    ///
    /// # Safety
    ///
    /// The thread must stay blocked while this runs.
    pub unsafe fn user_state(
        &self,
    ) -> Option<(
        crate::arch::exception::TrapFrame,
        u64,
        crate::arch::context::FpState,
    )> {
        use crate::arch::exception::TrapFrame;
        let fp = self.fp.as_ref()?;
        if !self.state().is_blocked() {
            return None;
        }
        // It may still be switching out on another CPU: wait until its
        // context is saved.
        while self.on_cpu.load(Ordering::Acquire) != 0 {
            core::hint::spin_loop();
        }
        let at = self.kstack_top - core::mem::size_of::<TrapFrame>() as u64;
        // SAFETY: a user thread entered the kernel from EL0, so its trap
        // frame is at the top of its kernel stack.
        let frame = unsafe { core::ptr::read(at as *const TrapFrame) };
        // SAFETY: it is blocked, so its context is saved and not changing.
        let tp = unsafe { (*self.context.get()).user_tp() };
        // SAFETY: as for the context: its FP state is saved.
        let fp_state = unsafe { (*fp.get()).clone() };
        Some((frame, tp, fp_state))
    }
}

/// First Rust code of a new thread (called from `thread_start`).
#[unsafe(no_mangle)]
extern "C" fn thread_entry(entry: usize, arg: usize) -> ! {
    started();
    arch::irq_enable();
    // SAFETY: `spawn` stored a `fn(usize)` as the entry.
    let entry: fn(usize) = unsafe { core::mem::transmute(entry) };
    entry(arg);
    exit()
}

/// End the calling thread.
pub fn exit() -> ! {
    let me = current();
    // The last thread of a debugged process waits for its debugger.
    if me.process.thread_count() == 1 && me.process.pid != 1 {
        crate::ker::debug::park_at_exit(me);
    }
    me.process.add_cpu_time(me.cpu_time());
    crate::ker::signal::forget_thread(&me.process, me.ptid);
    crate::ker::msg::thread_died(me);
    crate::ker::thread::exit_event(me);
    crate::sporadic::clear(me);
    crate::ker::trace::emit(
        crate::ker::trace::C_PR_TH,
        crate::ker::trace::TH_DESTROY,
        &[me.process.pid as u32, me.ptid],
    );
    if me.process.thread_exited() && me.process.pid != 1 {
        // Once it is a zombie, its parent's wait or process 1's exit work can
        // `process::remove` it, which frees the address space as soon as
        // its threads are off their CPUs (`wait_off_cpu`). This thread
        // still runs on those tables: it must not leave its CPU before its
        // final switch (review IPC-12). A user thread's kernel paths run
        // with IRQs masked, so only a preemption point could switch it:
        // none from here on.
        me.kcall_preemptible.store(false, Ordering::Relaxed);
        let daif = arch::irq_save();
        arch::irq_restore(daif);
        debug_assert!(daif & 1 << 7 != 0, "a process's last thread ends with IRQs enabled");
        crate::ker::process::last_thread_exited(&me.process);
    }
    // From here on not preemptible (process 1's threads run with IRQs
    // enabled): preempted while on the dead list but not yet dead, the
    // thread would be queued as ready, and `reap` could free its stack
    // before it runs again. The switch away unmasks.
    let _ = arch::irq_save();
    if me.stack.1 != 0 {
        DEAD.lock().push(me.arc());
    }
    {
        // Dead, then its joiner woken: `ThreadJoin()` checks both under
        // the same lock.
        let _g = JOIN_LOCK.lock();
        me.state.store(State::Dead as u8, Ordering::Release);
        if let Some(j) = me.joiner.lock().take() {
            wake(&j);
        }
        // Nobody will join it: its ID is free.
        if me.detached.load(Ordering::Acquire) {
            me.process.release_ptid(me.ptid);
        }
    }
    schedule(Reason::Exit);
    unreachable!("dead thread rescheduled");
}

/// Give the CPU to another ready thread of the same priority, if any.
pub fn yield_now() {
    schedule(Reason::Yield);
}

/// Load the calling thread's FP/SIMD state into the registers if it isn't
/// there yet (its first FP instruction since it was switched in, or kernel
/// code about to use its registers), and let EL0 use them.
pub fn fp_make_live() {
    let me = current();
    let daif = irq_save();
    if !me.fp_live.load(Ordering::Relaxed)
        && let Some(fp) = &me.fp
    {
        // SAFETY: the running thread's own state.
        crate::arch::context::fp_restore(unsafe { &*fp.get() });
        me.fp_live.store(true, Ordering::Relaxed);
    }
    crate::arch::context::fp_set_el0_trap(false);
    irq_restore(daif);
}

/// Mark the calling thread blocked in `state`. Call with the lock of the
/// object it waits on held, then release that lock and call `block`. The
/// error a wake may leave (`wake_error`) starts at 0 for every block, so
/// one left unread by an earlier wait never ends this one (review
/// SCHED-21).
pub fn prepare_block(state: State) {
    debug_assert!(state.is_blocked());
    current().wake_error.store(0, Ordering::Relaxed);
    current().state.store(state as u8, Ordering::Release);
    crate::ker::trace::thread_state(current(), state);
}

/// Wait until woken (after `prepare_block`).
pub fn block() {
    schedule(Reason::Block);
}

/// Change a blocked thread's state (e.g. SEND- to REPLY-blocked) without
/// waking it. Call with the lock of the object it waits on held.
pub fn set_blocked_state(t: &Thread, state: State) {
    debug_assert!(state.is_blocked() && t.state().is_blocked());
    t.state.store(state as u8, Ordering::Release);
    crate::ker::trace::thread_state(t, state);
}

/// Make a blocked thread ready. Returns false if it was not blocked.
pub fn wake(t: &Thread) -> bool {
    wake_as(t, None)
}

/// Make a blocked thread ready, leaving its kernel call with `err`
/// (`wake_error`). Returns false, and leaves `wake_error` alone, if it was
/// not blocked: another wake came first, and its reason stands (a signal
/// that woke a `sigwaitinfo()` isn't turned into a timeout).
pub fn wake_with(t: &Thread, err: u32) -> bool {
    wake_as(t, Some(err))
}

// ANCHOR: wake
fn wake_as(t: &Thread, err: Option<u32>) -> bool {
    let mut q = READY.lock();
    // Every wake holds the ready queue's lock: a thread seen blocked here
    // stays blocked until this wake, but may move between blocked states
    // (`set_blocked_state`), so the exchange is retried.
    loop {
        let s = t.state.load(Ordering::Acquire);
        if !State::from_u8(s).is_blocked() {
            return false;
        }
        if let Some(e) = err {
            t.wake_error.store(e, Ordering::Relaxed);
        }
        if t.state
            .compare_exchange(s, State::Ready as u8, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            break;
        }
    }
    q.push(t.arc(), false);
    crate::ker::trace::thread_state(t, State::Ready);
    let higher = t.priority() > current().priority();
    drop(q);
    if t.policy() == Policy::Sporadic {
        crate::sporadic::woken(t, crate::clock::now_ns());
    }
    if higher {
        percpu::this()
            .sched
            .need_resched
            .store(true, Ordering::Relaxed);
    }
    kick_for(t, higher);
    true
}
// ANCHOR_END: wake

// ANCHOR: kick_for
/// Thread `t` has just become ready: interrupt the CPU that should run it
/// now, if another than this one (an idle CPU of its runmask first, else
/// the one running the lowest priority below `t`'s). `local` is whether
/// this CPU will switch to it already.
fn kick_for(t: &Thread, local: bool) {
    let me = percpu::this().cpu;
    let prio = t.priority();
    let mut best: Option<(u32, u8)> = None;
    for cpu in (0..crate::percpu::MAX_CPUS as u32).filter(|&c| c != me && t.may_run_on(c)) {
        let Some(c) = percpu::get(cpu).filter(|c| c.online.load(Ordering::Acquire)) else {
            continue;
        };
        let running = c.running_prio.load(Ordering::Relaxed);
        // An idle CPU already sent for takes the thread it was sent for.
        if running == PRIO_IDLE && c.kicked.load(Ordering::Relaxed) {
            continue;
        }
        if running < prio && best.is_none_or(|(_, b)| running < b) {
            best = Some((cpu, running));
        }
    }
    match best {
        // An idle CPU takes it even if this one would.
        Some((cpu, PRIO_IDLE)) => {
            if let Some(c) = percpu::get(cpu) {
                c.kicked.store(true, Ordering::Relaxed);
            }
            crate::ipi::send(cpu, crate::ipi::RESCHED)
        }
        Some((cpu, _)) if !(local && t.may_run_on(me)) => crate::ipi::send(cpu, crate::ipi::RESCHED),
        _ => {}
    }
}
// ANCHOR_END: kick_for

/// Make thread `t`, if it is running on another CPU, enter the kernel
/// (and so pass `ker::user_return` on its way back to EL0).
pub fn interrupt(t: &Thread) {
    let cpu = t.last_cpu() as u32;
    if t.state() == State::Running && cpu != percpu::this().cpu {
        crate::ipi::send(cpu, crate::ipi::RESCHED);
    }
}

/// An interprocessor interrupt asks this CPU to reschedule (`ipi`).
pub fn request_resched() {
    percpu::this()
        .sched
        .need_resched
        .store(true, Ordering::Relaxed);
}

/// Set the priority thread `t` runs at by itself (`SchedSet()`); it takes
/// effect at once, unless a mutex it holds raises it above that
/// (`ker::sync::own_priority_changed`).
pub fn set_base_priority(t: &Thread, prio: u8) {
    t.base_prio.store(prio, Ordering::Relaxed);
    set_priority(t, crate::ker::sync::own_priority_changed(t, prio));
}

/// Change a thread's effective priority (message-driven priority
/// inheritance), requeueing it if it is ready.

pub fn set_priority(t: &Thread, prio: u8) {
    if t.priority() == prio {
        return; // the common case (message-driven inheritance), unlocked
    }
    // Lowered while it runs on another CPU: that CPU looks again at what
    // it should run, once the new priority is stored (review SCHED-10;
    // procnto sends an IPI).
    let lowered_elsewhere = !core::ptr::eq(t, current()) && prio < t.priority();
    if core::ptr::eq(t, current()) {
        percpu::this().running_prio.store(prio, Ordering::Relaxed);
        crate::clock::priority_lowered();
    }
    let mut q = READY.lock();
    if t.priority() == prio {
        return;
    }
    let queued = t.state() == State::Ready && q.remove(t);
    t.prio.store(prio, Ordering::Relaxed);
    if queued {
        q.push(t.arc(), false);
    }
    // The running thread may now be below a ready one.
    let local = q
        .highest_for(percpu::this().cpu)
        .is_some_and(|p| p > current().priority());
    drop(q);
    if local {
        percpu::this()
            .sched
            .need_resched
            .store(true, Ordering::Relaxed);
    }
    // A ready thread raised above what another CPU runs goes there.
    if queued {
        kick_for(t, local);
    }
    if lowered_elsewhere {
        interrupt(t);
    }
}

/// A tick on the CPU running round-robin thread `t`: true if it reached
/// its timeslice's end (the next one is a slice on).
fn slice_tick(t: &Thread) -> bool {
    let ran = t.cpu_time();
    if ran < t.slice_end.load(Ordering::Relaxed) {
        return false;
    }
    t.slice_end.store(ran + rr_slice_ns(), Ordering::Relaxed);
    true
}

/// `t` runs on this CPU from monotonic time `now` with `ran` ns of run
/// time: a sporadic thread's budget starts, if it hasn't.
fn start_slice(t: &Thread, now: u64, ran: u64) {
    if t.policy() == Policy::Sporadic {
        crate::sporadic::start(t, now, ran);
    }
}

/// Thread `t`'s timeslice or budget changed: if it is running, its CPU
/// looks again (passing through `schedule`).
pub fn reassess(t: &Thread) {
    if core::ptr::eq(t, current()) {
        percpu::this().sched.need_resched.store(true, Ordering::Relaxed);
    } else {
        interrupt(t);
    }
}

/// Clock tick on this CPU: ask for a switch if the round-robin thread it
/// runs used up its timeslice, or a higher-priority thread is waiting; a
/// sporadic thread that used up its budget drops to its low priority.
pub fn tick() {
    let me = percpu::this();
    let cur = current();
    if cur.policy() == Policy::Sporadic {
        crate::sporadic::budget_check(cur);
    } else if cur.policy().sliced() && !core::ptr::eq(cur, me.sched.idle.load(Ordering::Relaxed)) && slice_tick(cur) {
        cur.slice_out.store(true, Ordering::Relaxed);
        me.sched.need_resched.store(true, Ordering::Relaxed);
    }
    let higher = READY
        .lock()
        .highest_for(percpu::this().cpu)
        .is_some_and(|p| p > cur.priority());
    if higher {
        me.sched.need_resched.store(true, Ordering::Relaxed);
    }
}

/// A point in a long kernel operation (a message copy) with no lock held:
/// interrupts are taken for a moment, then the thread switches if a
/// higher-priority one is ready for this CPU (review IPC-10, CORE-6). Only
/// in a kernel call whose caller had interrupts enabled
/// (`Thread::kcall_preemptible`); elsewhere it does nothing.
pub fn preemption_point() {
    if !current().kcall_preemptible.load(Ordering::Relaxed) {
        return;
    }
    let daif = irq_save();
    arch::irq_window();
    irq_restore(daif);
    preempt();
}

/// Called on the way out of an interrupt or kernel call: switch if asked.
pub fn preempt() {
    let me = percpu::this();
    if me.sched.preemptible.load(Ordering::Relaxed)
        && me.sched.need_resched.swap(false, Ordering::Relaxed)
    {
        schedule(Reason::Preempt);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// Timeslice expired or a higher priority is ready.
    Preempt,
    Yield,
    Block,
    Exit,
}

fn schedule(reason: Reason) {
    let daif = irq_save();
    let me = percpu::this();
    me.kicked.store(false, Ordering::Relaxed);
    let cur = current();
    let idle = me.sched.idle.load(Ordering::Relaxed);
    let cur_is_idle = core::ptr::eq(cur, idle);

    let mut q = READY.lock();
    // Woken after it blocked but before it switched away (blocking, or
    // preempted on the way): still queued, it runs on (left queued, it
    // could be picked here as the next thread, and wait for itself); taken
    // by another CPU already, it goes there (it is Ready, and switched
    // away from below).
    if !cur_is_idle && cur.state() == State::Ready && q.remove(cur) {
        cur.state.store(State::Running as u8, Ordering::Relaxed);
        if reason == Reason::Block {
            drop(q);
            irq_restore(daif);
            return;
        }
    }
    // A running thread whose runmask no longer has this CPU goes back to
    // the queue, for another CPU.
    let running = cur.state() == State::Running && !cur_is_idle;
    let runnable = running && cur.may_run_on(me.cpu);
    let prio = cur.priority();
    let slice_out = cur.slice_out.swap(false, Ordering::Relaxed);
    // ANCHOR: schedule_choice
    let next = match q.highest_for(me.cpu) {
        Some(p) if !runnable || cur_is_idle => q.pop_for(p, me.cpu),
        Some(p) if p > prio => q.pop_for(p, me.cpu),
        Some(p)
            if p == prio && (reason == Reason::Yield || slice_out) =>
        {
            q.pop_for(p, me.cpu)
        }
        _ => None,
    };
    // ANCHOR_END: schedule_choice
    // No counted reference may stay in this frame across the switch: a
    // dead thread's never returns. The thread table keeps `next`.
    let next: &Thread = match next {
        Some(n) => {
            if running {
                // Preempted by a higher priority: back to the head of the
                // queue, unless its timeslice ended at the same tick (then
                // the tail, as procnto: a thread woken on every tick would
                // otherwise keep the others of its priority from running).
                let front = n.priority() > prio && reason == Reason::Preempt && !slice_out;
                cur.state.store(State::Ready as u8, Ordering::Relaxed);
                crate::ker::trace::thread_state(cur, State::Ready);
                q.push(cur.arc(), front);
            }
            let p = Arc::as_ptr(&n);
            drop(n);
            // SAFETY: a ready thread is in the thread table.
            unsafe { &*p }
        }
        None if runnable => {
            // Keep running (a sporadic budget starts again if it was
            // replenished).
            if cur.policy() == Policy::Sporadic {
                start_slice(cur, crate::clock::now_ns(), cur.cpu_time());
            }
            drop(q);
            irq_restore(daif);
            return;
        }
        // Nothing else to run: idle (or stay idle).
        None if cur_is_idle => {
            drop(q);
            irq_restore(daif);
            return;
        }
        None => {
            if running {
                cur.state.store(State::Ready as u8, Ordering::Relaxed);
                crate::ker::trace::thread_state(cur, State::Ready);
                q.push(cur.arc(), false);
            }
            // SAFETY: idle threads live forever.
            unsafe { &*idle }
        }
    };
    // CPU time: `cur`'s run ends, under the queue's lock, so before
    // another CPU can pick it and start its next run.
    let now = crate::clock::now_ns();
    let ran = now.saturating_sub(cur.run_since.load(Ordering::Relaxed));
    cur.cpu_ns.fetch_add(ran, Ordering::Release);
    cur.run_since.store(NOT_RUNNING, Ordering::Release);
    drop(q);
    // `cur` went back to the queue (preempted, or no longer allowed here):
    // another CPU may run it now.
    if running {
        kick_for(cur, false);
    }
    // A sporadic thread that blocks ends a chunk of its budget.
    if !running && !cur_is_idle && cur.policy() == Policy::Sporadic && cur.state().is_blocked() {
        crate::sporadic::blocked(cur, cur.cpu_ns.load(Ordering::Relaxed));
    }
    reap();

    me.running_prio.store(
        if core::ptr::eq(next, idle) { PRIO_IDLE } else { next.priority() },
        Ordering::Relaxed,
    );
    crate::clock::priority_lowered();
    // `next` stays Ready until it runs here (`resumed`): the CPU still
    // leaving it then knows that another took it, and leaves it. Its run
    // starts there too, once that CPU has switched away from it and ended
    // the run it had (above, under the queue's lock): started here, the
    // other CPU could take this start for its own and then clear it.
    next.cpus_seen.fetch_or(1 << me.cpu, Ordering::Relaxed);
    next.last_cpu.store(me.cpu as u8, Ordering::Relaxed);
    me.sched
        .current
        .store(next as *const Thread as *mut Thread, Ordering::Release);
    // Stepping is for one thread's return to EL0 only (`ker::debug`).
    crate::ker::debug::set_step(false);
    // User FP/SIMD state, lazily: saved only if `cur` used it since it was
    // switched in (the kernel never touches these registers, so they still
    // hold its values); `next` loads its own at its first FP instruction.
    if cur.fp_live.swap(false, Ordering::Relaxed)
        && let Some(fp) = &cur.fp
    {
        // SAFETY: only the CPU running `cur` accesses its FP state.
        crate::arch::context::fp_save(unsafe { &mut *fp.get() });
    }
    crate::arch::context::fp_set_el0_trap(true);
    // process 1's threads (EL1) run on whichever user tables are loaded, as
    // QNX's do: a message to procnto and its reply switch no tables.
    if !core::ptr::eq(&*next.process, &crate::ker::process::KERNEL) {
        let ttbr0 = next.process.aspace().ttbr0();
        if arch::uspace::loaded() != ttbr0 {
            arch::uspace::activate(ttbr0);
        }
    }
    // SAFETY: IRQs are masked; `cur` is running here, and `next` is
    // switched to once the CPU that ran it last has saved it.
    // `next.on_cpu` is waited for and set in the switch, once `cur` is
    // saved (see `context::switch`).
    unsafe {
        context::switch(
            cur.context.get(),
            next.context.get(),
            cur.on_cpu.as_ptr(),
            next.on_cpu.as_ptr(),
        )
    };
    // Back in `cur`, possibly on another CPU.
    resumed();
    irq_restore(daif);
}

/// The calling thread has just been switched to (back from the switch, or
/// at its first instruction): it is Running. Until now it was Ready (out
/// of the queue), so that the CPU that was leaving it, if it was still
/// running there, saw that another CPU had taken it (`schedule`).
fn resumed() {
    let me = current();
    // Its run starts (CPU time, a sporadic budget).
    let now = crate::clock::now_ns();
    start_slice(me, now, me.cpu_ns.load(Ordering::Acquire));
    me.run_since.store(now, Ordering::Release);
    me.state.store(State::Running as u8, Ordering::Relaxed);
    crate::ker::trace::thread_state(me, State::Running);
}

/// `resumed`, for a new thread's first instruction.
pub fn started() {
    resumed();
}

/// Dead threads whose kernel stacks are still to be freed.
static DEAD: IrqSpinLock<Vec<ThreadRef>> = IrqSpinLock::new(Vec::new());

/// Free the dead threads that no CPU runs on any more: their kernel
/// stacks now, the rest with the last reference to them. A thread that
/// can still be joined stays in the thread table (a zombie, for
/// `ThreadJoin()`) until it is (`forget`) or its process goes
/// (`forget_process`).
fn reap() {
    let mut gone = Vec::new();
    {
        let mut dead = DEAD.lock();
        let mut i = 0;
        while i < dead.len() {
            // Off its CPU, and dead (not between `exit`'s steps).
            if dead[i].on_cpu.load(Ordering::Acquire) != 0 || dead[i].state() != State::Dead {
                i += 1;
                continue;
            }
            let t = dead.swap_remove(i);
            let (base, pages) = t.stack;
            if pages != 0 {
                kmem::unmap_guarded(base, pages);
            }
            // Decided under the lock `forget` checks.
            let joinable = !t.detached.load(Ordering::Acquire)
                && !t.joined.load(Ordering::Acquire)
                && t.process.pid != crate::ker::process::KERNEL_PID;
            if !joinable {
                gone.push(t);
            }
        }
    }
    if gone.is_empty() {
        return;
    }
    let mut all = THREADS.lock();
    for t in &gone {
        all.remove(&t.tid);
    }
    drop(all);
    // Freed here unless something still holds them.
    drop(gone);
}

/// Dead thread `t` has been joined: it can go (now, or when `reap` gets
/// to it).
pub fn forget(t: &Thread) {
    let dead = DEAD.lock();
    if dead.iter().any(|d| core::ptr::eq(&**d, t)) {
        return;
    }
    let gone = THREADS.lock().remove(&t.tid);
    drop(dead);
    drop(gone);
}

/// Process `p` is gone: so are its threads that were never joined.
pub fn forget_process(p: &crate::ker::process::Process) {
    let mut gone = Vec::new();
    THREADS.lock().retain(|_, t| {
        let keep = !core::ptr::eq(&*t.process, p) || t.state() != State::Dead;
        if !keep {
            gone.push(t.clone());
        }
        keep
    });
    drop(gone);
}

/// Idle loop of a CPU: run whatever is ready, otherwise wait for interrupts.
/// CPUs stop their tick while they wait (`clock::stop_tick`; each keeps
/// the one when its next timeout or timer is due): a thread made ready for
/// them comes with an IPI (`kick_for`).
pub fn idle() -> ! {
    let me = percpu::this();
    me.sched.preemptible.store(true, Ordering::Relaxed);
    loop {
        schedule(Reason::Preempt);
        // Masked from the check to the wait: an IPI after the check stays
        // pending and ends the wait at once (WFI wakes on it, masked).
        let daif = irq_save();
        let work = me.sched.need_resched.load(Ordering::Relaxed)
            || READY.lock().highest_for(me.cpu).is_some();
        if !work && crate::clock::stop_tick() {
            arch::wait_for_interrupt();
            crate::clock::restart_tick();
        }
        irq_restore(daif);
    }
}

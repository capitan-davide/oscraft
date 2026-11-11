//! Processes, as far as the kernel needs them: an address space, threads,
//! and the owner of channels and connections, which are named by small
//! per-process integers.
//!
//! Process 1 is the kernel itself; its threads run in the kernel and its user
//! address space is empty. Every other process starts with a connection to
//! process 1's system-manager channel as `SYSMGR_COID` (`_NTO_SIDE_CHANNEL`),
//! as in QNX. A channel lives while its process's table or a connection
//! (or a thread blocked on it) holds it; `ChannelDestroy`, or the
//! process's end, empties it.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicU64, Ordering};

use super::channel::{self, Channel, Connection};
use super::signal::Signals;
use crate::arch::uspace::AddressSpace;
use crate::sync::{InitOnce, IrqSpinLock};
use crate::vm::map::MemMap;

/// `_NTO_SIDE_CHANNEL`: connection IDs from here on are "side channels",
/// outside the file-descriptor space.
pub const SIDE_CHANNEL: u32 = 0x4000_0000;
/// process 1's process ID (`SYSMGR_PID`).
pub const KERNEL_PID: i32 = 1;
/// process 1's system-manager channel ID.
pub const SYSMGR_CHID: i32 = 1;
/// procnto's text console channel (QNX `/dev/text`); new processes get
/// their stdin, stdout and stderr (connections 0-2) on it for now.
pub const DEVTEXT_CHID: i32 = 2;

pub use crate::personality::Personality;

pub struct Process {
    pub pid: i32,
    /// The ABI of its program (`Personality`).
    personality: AtomicU8,
    /// Parent process (procnto for those the boot script starts).
    pub ppid: AtomicI32,
    /// How it ended: (`si_code`, `si_status`) of its SIGCHLD; set by
    /// `exit_current`, or (CLD_EXITED, 0) if its threads just ended;
    /// (CLD_KILLED, 0) once its debugger let go of it at its end
    /// (`debug::park_at_exit`).
    pub exit: IrqSpinLock<Option<(i32, i32)>>,
    /// All its threads have ended; it waits for its parent's wait().
    zombie: AtomicBool,
    /// Its thread is doing the loader's work (the `_IO_DUP`s after a fork):
    /// servers see `_NTO_CI_LOADER` in its client info.
    pub loading: AtomicBool,
    /// Severed from its parent (`_NTO_PF_NOZOMBIE`): its parent has seen
    /// it end already, so it leaves no zombie and sends no SIGCHLD.
    pub nozombie: AtomicBool,
    /// Copies in flight into or out of its memory (`umem::copy`): its
    /// private frames unmapped meanwhile are freed only once this is 0, as
    /// such a copy may have translated a page before the unmap (review
    /// IPC-2: a frame freed and reused under a client's copy).
    pub copies: AtomicU32,
    /// Ended while its parent ignored SIGCHLD: no wait reports it.
    pub unwaited: AtomicBool,
    /// Runs a program of its own (spawned, or it has exec'ed): its parent
    /// can no longer move it to another process group (`setpgid()`:
    /// EACCES, POSIX and procnto, measured: review PROCMGR-10).
    pub execed: AtomicBool,
    /// File creation mask (`umask()`), procnto's to keep.
    pub umask: AtomicU32,
    /// A signal may be pending (set when one is posted, cleared when none
    /// is): lets the return to EL0 skip the signal lock.
    pub sig_hint: AtomicBool,
    /// The process may be stopped (set when a stop begins, never cleared):
    /// lets the return to EL0 skip the debug lock.
    pub stop_hint: AtomicBool,
    /// User and group IDs, see `Cred`.
    pub cred: IrqSpinLock<Cred>,
    /// Process group and session (0 until set: the process's own).
    pgid: AtomicI32,
    sid: AtomicI32,
    /// Live threads.
    threads: AtomicU32,
    /// The QNX thread IDs `ThreadCreate()` gave that are in use: those of
    /// threads alive or not yet joined (the first thread is 1, always).
    ptids: IrqSpinLock<alloc::collections::BTreeSet<u32>>,
    /// Process-local storage pointer set by libc (`__SysCpupageSet`).
    pub pls: AtomicU64,
    /// When it was created (`clock::now_ns()`).
    pub start_time: AtomicU64,
    /// Nanoseconds its ended threads ran; those its ended children ran.
    cpu_ns: AtomicU64,
    child_cpu_ns: AtomicU64,
    /// Where its program is loaded, and its first thread's initial stack
    /// pointer (at argc), for introspection.
    pub base_address: AtomicU64,
    pub initial_stack: AtomicU64,
    /// The path of the program it runs, without the leading '/' (QNX's
    /// `debug_name`, which `_PROC_GETNAME` and pidin show).
    pub name: IrqSpinLock<String>,
    aspace: InitOnce<AddressSpace>,
    /// Working directory (absolute, without the leading '/'), see
    /// `proc::pathmgr`.
    pub cwd: IrqSpinLock<String>,
    /// Root directory (`chroot()`; absolute, without the leading '/', ""
    /// for the real one), see `proc::pathmgr`.
    pub root: IrqSpinLock<String>,
    /// Signal actions, see `ker::signal`.
    pub signals: IrqSpinLock<Signals>,
    /// Job-control and debugger stops, see `ker::debug`.
    pub debug: IrqSpinLock<super::debug::DebugState>,
    /// Registered events (`MsgRegisterEvent()`), indexed by handle: the
    /// event and the pid allowed to deliver it (-1: any), see `ker::event`.
    pub events: IrqSpinLock<Vec<Option<(super::event::SigEvent, i32)>>>,
    /// The regions of the address space (`mmap()` and the loader's).
    pub mem: IrqSpinLock<MemMap>,
    /// Resource limits set (`setrlimit()`): resource -> (current,
    /// maximum); the others are at their defaults (see `procmgr`).
    pub limits: IrqSpinLock<BTreeMap<u32, (u64, u64)>>,
    /// Its timers' default tolerance in ns (`procmgr_timer_tolerance()`;
    /// 0: none). How late its timers may fire, at most; kept, not used:
    /// a timer is high-resolution only with its own `TIMER_TOLERANCE`
    /// (as procnto, see hrtest), and none fires later than its tick.
    pub timer_tolerance: AtomicU64,
    /// Added to its CPU-time clock's reading since it was set
    /// (`clock_settime(CLOCK_PROCESS_CPUTIME_ID)`), wrapping.
    pub cpu_clock_offset: AtomicU64,
    /// Its abilities (`procmgr_ability()`), see `ability`.
    pub abilities: IrqSpinLock<super::ability::Abilities>,
    /// Indexed by channel ID; ID 0 is never used.
    channels: IrqSpinLock<Vec<Option<Arc<Channel>>>>,
    /// The IDs of the kernel message queues it made as a manager: slots
    /// of the channel-ID space, as procnto's (`mqueue::msg_queue_open`).
    /// Locked only with `channels` held.
    queue_ids: IrqSpinLock<alloc::collections::BTreeSet<i32>>,
    /// Indexed by connection ID (file-descriptor space).
    connections: IrqSpinLock<Vec<Option<Arc<Connection>>>>,
    /// Indexed by connection ID - SIDE_CHANNEL.
    side_connections: IrqSpinLock<Vec<Option<Arc<Connection>>>>,
    /// Itself, for `arc()` (none for process 1).
    this: Weak<Process>,
}

pub static KERNEL: Process = Process::new(KERNEL_PID);

/// A counted reference to a process: the process table holds one until
/// the process is removed (`remove`), each of its threads one, and the
/// process is freed with the last. process 1's is static. Code running on
/// behalf of a process uses `current()`, a plain reference.
#[derive(Clone)]
pub struct ProcessRef(Option<Arc<Process>>);

impl ProcessRef {
    pub fn kernel() -> Self {
        Self(None)
    }
}

impl core::ops::Deref for ProcessRef {
    type Target = Process;

    fn deref(&self) -> &Process {
        self.0.as_deref().unwrap_or(&KERNEL)
    }
}

/// The processes (procnto included), in slot order, as procnto's process
/// vector (what `/proc` lists).
static PROCESSES: IrqSpinLock<Vec<ProcessRef>> = IrqSpinLock::new(Vec::new());

/// The low bits of a pid: its slot in the process table.
const PID_MASK: i32 = 0xfff;

/// The process table's slots and generation, as procnto's: a new process
/// takes the lowest free slot (procnto has 1; 0 is never used), and its pid
/// is that slot plus the generation, which grows by `PID_MASK + 1` each
/// time a process is forgotten (`remove`), staying positive (measured; the
/// `pid_unique` of openQNX's `kerext_process_destroy`). A zombie keeps its
/// slot until it is waited for.
struct Pids {
    used: [u64; (PID_MASK as usize + 1) / 64],
    unique: i32,
}

static PIDS: IrqSpinLock<Pids> = IrqSpinLock::new(Pids {
    // Slot 0 and procnto's.
    used: {
        let mut used = [0; (PID_MASK as usize + 1) / 64];
        used[0] = 0b11;
        used
    },
    unique: 0,
});

/// Create process 1's (empty) user address space and its system-manager
/// channel. Needs the heap.
pub fn init() {
    let aspace = AddressSpace::new().expect("kernel address space");
    assert!(
        KERNEL.aspace.set(aspace).is_ok(),
        "process::init called twice"
    );
    // `_NTO_CHF_UNBLOCK`, as procnto's: a client it holds (a wait) leaves
    // on a signal when procnto answers it (EINTR), not before.
    const CHF_UNBLOCK: u32 = 0x2;
    assert_eq!(channel::channel_create_in(&KERNEL, CHF_UNBLOCK), SYSMGR_CHID);
    assert_eq!(channel::channel_create_in(&KERNEL, 0), DEVTEXT_CHID);
    *KERNEL.name.lock() = "kernel".into();
    PROCESSES.lock().push(ProcessRef::kernel());
}

/// A new process structure with an empty address space and the next pid
/// (`PIDS`), started now. None without memory or a free slot.
fn new_process() -> Option<ProcessRef> {
    let aspace = AddressSpace::new()?;
    let pid = {
        let mut pids = PIDS.lock();
        let slot = (0..pids.used.len())
            .find(|&w| pids.used[w] != u64::MAX)
            .map(|w| w * 64 + pids.used[w].trailing_ones() as usize)?;
        pids.used[slot / 64] |= 1 << (slot % 64);
        slot as i32 | pids.unique
    };
    let p = Arc::new_cyclic(|this| Process {
        this: this.clone(),
        ..Process::new(pid)
    });
    let _ = p.aspace.set(aspace);
    p.start_time
        .store(crate::clock::now_ns(), Ordering::Relaxed);
    Some(ProcessRef(Some(p)))
}

/// Make process `p` known, in slot order.
fn publish(p: &ProcessRef) {
    // On the heap: 128 actions don't belong on a kernel stack.
    // Process 1, static, gets its table with the first one.
    for q in [&**p, &KERNEL] {
        let mut sig = q.signals.lock();
        if sig.actions.is_empty() {
            sig.actions.resize(super::signal::NSIG, Default::default());
        }
    }
    let mut all = PROCESSES.lock();
    let i = all.partition_point(|q| q.pid & PID_MASK < p.pid & PID_MASK);
    all.insert(i, p.clone());
}

/// A new user process with an empty address space, connected to process 1.
pub fn create() -> Option<ProcessRef> {
    let p = new_process()?;
    let sysmgr = KERNEL.channel(SYSMGR_CHID)?;
    p.add_connection(SIDE_CHANNEL, Connection::attach(sysmgr, p.pid))?;
    let text = KERNEL.channel(DEVTEXT_CHID)?;
    for fd in 0..3 {
        p.add_connection(fd, Connection::attach(text.clone(), p.pid))?;
    }
    publish(&p);
    Some(p)
}

/// A copy of `parent` for `fork()`: its file descriptors (with new server
/// connection IDs), working directory, signal actions and process-local
/// storage pointer. Its address space is empty; the caller copies the
/// memory (`vm::map::fork`). Channels are not inherited, and of the side
/// channel connections only procnto's, as procnto's `loader_fd_inherit()`
/// copies file descriptors only.
pub fn fork(parent: &Process) -> Option<crate::ker::process::ProcessRef> {
    let p = new_process()?;
    p.ppid.store(parent.pid, Ordering::Relaxed);
    p.pgid.store(parent.pgid(), Ordering::Relaxed);
    p.sid.store(parent.sid(), Ordering::Relaxed);
    p.umask
        .store(parent.umask.load(Ordering::Relaxed), Ordering::Relaxed);
    *p.cred.lock() = parent.cred.lock().clone();
    *p.limits.lock() = parent.limits.lock().clone();
    *p.abilities.lock() = parent.abilities.lock().clone();
    p.timer_tolerance
        .store(parent.timer_tolerance.load(Ordering::Relaxed), Ordering::Relaxed);
    for (table, base) in [
        (&parent.connections, 0),
        (&parent.side_connections, SIDE_CHANNEL),
    ] {
        let conns: Vec<(usize, Arc<Connection>)> = table
            .lock()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((i, c.clone()?)))
            .filter(|(_, c)| base == 0 || c.channel.owner.pid == KERNEL_PID)
            .collect();
        for (i, c) in conns {
            let dup = c.copy_for(p.pid);
            dup.flags
                .store(c.flags.load(Ordering::Relaxed), Ordering::Relaxed);
            let _ = p.add_connection(base + i as u32, dup); // the parent's fit
        }
    }
    *p.cwd.lock() = parent.cwd.lock().clone();
    *p.root.lock() = parent.root.lock().clone();
    *p.signals.lock() = parent.signals.lock().clone();
    *p.name.lock() = parent.name.lock().clone();
    super::trace::process_created(&p);
    super::trace::process_named(&p);
    p.set_personality(parent.personality());
    for (to, from) in [
        (&p.pls, &parent.pls),
        (&p.base_address, &parent.base_address),
        (&p.initial_stack, &parent.initial_stack),
    ] {
        to.store(from.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    publish(&p);
    Some(p)
}

/// A new process for `spawn()`, child of `parent`, in its session and
/// process group, with its working directory. Its file descriptors are
/// copies of the parent's: fd i is the parent's `fd_map[i]` (-1: none),
/// or, without a map, every one not marked close-on-exec. Returns the
/// process and the (parent fd, child fd) pairs copied.
pub fn spawn_child(
    parent: &Process,
    fd_map: Option<&[i32]>,
) -> Option<(ProcessRef, Vec<(i32, i32)>)> {
    let p = new_process()?;
    p.ppid.store(parent.pid, Ordering::Relaxed);
    p.pgid.store(parent.pgid(), Ordering::Relaxed);
    p.sid.store(parent.sid(), Ordering::Relaxed);
    p.umask
        .store(parent.umask.load(Ordering::Relaxed), Ordering::Relaxed);
    *p.cred.lock() = parent.cred.lock().clone();
    *p.limits.lock() = parent.limits.lock().clone();
    *p.abilities.lock() = parent.abilities.lock().inherited();
    *p.cwd.lock() = parent.cwd.lock().clone();
    *p.root.lock() = parent.root.lock().clone();
    super::trace::process_created(&p);
    let sysmgr = KERNEL.channel(SYSMGR_CHID)?;
    p.add_connection(SIDE_CHANNEL, Connection::attach(sysmgr, p.pid))?;
    let pairs: Vec<(i32, i32)> = match fd_map {
        Some(map) => map
            .iter()
            .enumerate()
            .filter(|(_, from)| **from >= 0)
            .map(|(to, &from)| (from, to as i32))
            .collect(),
        None => parent
            .connections
            .lock()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let c = c.as_ref()?;
                (c.flags.load(Ordering::Relaxed) & channel::COF_CLOEXEC == 0)
                    .then_some((i as i32, i as i32))
            })
            .collect(),
    };
    let mut copied = Vec::new();
    for (from, to) in pairs {
        if let Some(c) = parent.connection(from)
            && p.add_connection(to as u32, c.copy_for(p.pid)).is_some()
        {
            copied.push((from, to));
        }
    }
    publish(&p);
    Some((p, copied))
}

/// Pulse code (on process 1's system-manager channel) telling procnto that
/// process `value` has ended.
pub const PULSE_PROCESS_EXIT: i32 = 1;
/// Pulse code: forked child `value` has inherited its descriptors; its
/// parent's `fork()` can return.
pub const PULSE_FORK_DONE: i32 = 2;
/// A process stopped or continued (job control): process 1 answers waits.
pub const PULSE_JOB_CHANGED: i32 = 3;
/// Process `value` (low 32 bits) was severed from its parent (the high
/// 32 bits; `procmgr::detach`): process 1 answers the waits that settles.
pub const PULSE_DETACHED: i32 = 4;

// ANCHOR: exit_current
/// End the calling process: record how (`si_code`, `si_status`), make
/// its other threads end and end the calling thread.
pub fn exit_current(code: i32, status: i32) -> ! {
    let me = current();
    me.exit.lock().get_or_insert((code, status));
    // The other threads end too: each as it next returns to EL0 (at the
    // latest at the next clock tick), the blocked ones woken for that.
    let caller = crate::sched::current();
    for t in crate::sched::threads_of(me) {
        if !core::ptr::eq(&*t, caller) {
            t.exit_on_return.store(true, Ordering::Relaxed);
            if t.state().is_blocked() {
                super::unblock(&t, super::errno::EINTR);
            }
        }
    }
    crate::sched::exit()
}
// ANCHOR_END: exit_current

/// Called by the last thread of a user process as it ends: the process
/// becomes a zombie, its parent gets SIGCHLD, and process 1 is told (to
/// answer waits and free what it held).
pub fn last_thread_exited(p: &Process) {
    let (code, status) = p.exit.lock().unwrap_or((super::signal::CLD_EXITED, 0));
    // A parent that ignores SIGCHLD (or asked SA_NOCLDWAIT) waits for no
    // child: it leaves no zombie (POSIX, as procnto). Decided before it is
    // a zombie: a wait checked in between would take it as one.
    const SA_NOCLDWAIT: u32 = 0x20;
    if let Some(parent) = lookup(p.ppid.load(Ordering::Relaxed)) {
        let a = parent.signals.lock().actions[super::signal::SIGCHLD];
        if a.handler == super::signal::SIG_IGN || a.flags & SA_NOCLDWAIT != 0 {
            p.unwaited.store(true, Ordering::Release);
        }
    }
    // Its shared robust mutexes, through its own address space (the caller
    // is its last thread, but for a spawn that failed: it holds none).
    // Before it is a zombie and before the SIGCHLD: the parent's wait is
    // answered through them (`procmgr::unblocked`), and it must find the
    // mutexes marked when the wait returns.
    if core::ptr::eq(current(), p) {
        super::sync::process_exited(p);
    }
    p.zombie.store(true, Ordering::Release);
    if p.unwaited.load(Ordering::Relaxed) {
        p.nozombie.store(true, Ordering::Release);
    }
    super::trace::process_ended(p);
    let ppid = p.ppid.load(Ordering::Relaxed);
    if let Some(parent) = lookup(ppid) {
        let ns = p.cpu_time() + p.child_cpu_time();
        parent.child_cpu_ns.fetch_add(ns, Ordering::Relaxed);
    }
    // SIGCHLD (`si_uid` -1, as procnto sends it). A parent REPLY-blocked in
    // a wait isn't taken out of it by the signal: process 1's channel has
    // `_NTO_CHF_UNBLOCK`, and process 1 answers the wait (`procmgr::unblocked`).
    if ppid != KERNEL_PID
        && !p.nozombie.load(Ordering::Relaxed)
        && let Some(parent) = lookup(ppid)
    {
        let info = super::signal::SigInfo {
            code,
            pid: p.pid,
            uid: -1,
            status,
            ..Default::default()
        };
        super::signal::post(&parent, super::signal::SIGCHLD, info);
    }
    super::interrupt::process_exited(p);
    super::timer::process_exited(p);
    super::dupfd::process_exited(p.pid);
    // Straight to process 1's channel: a child that never ran ends in
    // process 1's own thread (a failed spawn), which has no side connection.
    if let Some(sysmgr) = KERNEL.channel(SYSMGR_CHID) {
        let prio = crate::sched::current().base_priority();
        super::msg::channel_pulse(&sysmgr, prio, PULSE_PROCESS_EXIT, p.pid as u64, 0);
    }
    // Its descriptors close: servers learn of the connections that go away
    // (`_PULSE_CODE_DISCONNECT`). Taken out of the tables first, dropped
    // without their locks held.
    let fds = core::mem::take(&mut *p.connections.lock());
    let side = core::mem::take(&mut *p.side_connections.lock());
    drop(fds);
    drop(side);
    // Its channels end: their clients are released (ESRCH), and they no
    // longer hold threads (and through them, the process).
    let channels = core::mem::take(&mut *p.channels.lock());
    for ch in channels.iter().flatten() {
        channel::destroy(ch);
    }
    // Processes connected to them learn which of their connections died:
    // a `_PULSE_CODE_COIDDEATH` pulse (the coid its value) on each of their
    // channels made with `_NTO_CHF_COID_DISCONNECT` (as procnto).
    const CHF_COID_DISCONNECT: u32 = 0x40;
    const PULSE_CODE_COIDDEATH: i32 = -35;
    // At the ending thread's priority, as procnto (measured: a process at
    // 12 ending; review IPC-14).
    let prio = crate::sched::current().priority();
    for q in all().into_iter().filter(|q| q.pid != p.pid && !q.is_zombie()) {
        let notify: Vec<Arc<Channel>> =
            q.channel_list().into_iter().filter(|c| c.flags & CHF_COID_DISCONNECT != 0).collect();
        if notify.is_empty() {
            continue;
        }
        for coid in q.connections_to(p.pid) {
            for ch in &notify {
                super::msg::channel_pulse(ch, prio, PULSE_CODE_COIDDEATH, coid as u64, 0);
            }
        }
    }
}

/// Forget a zombie process (its parent has waited for it).
pub fn remove(pid: i32) {
    let gone = {
        let mut all = PROCESSES.lock();
        let i = all.iter().position(|p| p.pid == pid);
        i.map(|i| all.remove(i))
    };
    if let Some(p) = &gone
        && p.pid != KERNEL_PID
    {
        channel::server_exited(p.pid);
        let mut pids = PIDS.lock();
        let slot = (p.pid & PID_MASK) as usize;
        pids.used[slot / 64] &= !(1 << (slot % 64));
        pids.unique = pids.unique.wrapping_add(PID_MASK + 1) & i32::MAX;
    }
    // Its address space goes too, once no CPU runs its threads any more
    // (its last thread may still be ending: `sched::exit` keeps it on its
    // CPU from the zombie mark to its final switch, so off the CPU means
    // gone for good).
    if let Some(p) = gone
        && p.pid != KERNEL_PID
        && let Some(aspace) = p.aspace.get()
    {
        crate::sched::wait_off_cpu(&p);
        // Its memory, unless process 1's exit work (`procmgr::process_exited`,
        // on the exit pulse) got to it first: a parent's wait can remove a
        // zombie before that.
        crate::vm::map::clear(&p);
        aspace.release();
        crate::sched::forget_process(&p);
    }
}

/// The client process behind server `server`'s connection ID `scoid`
/// (what a server sees in `_msg_info`).
pub fn by_scoid(server: i32, scoid: i32) -> Option<ProcessRef> {
    channel::scoid_client(server, scoid).and_then(lookup).filter(|_| server != 0)
}

/// All processes (procnto included), e.g. to find a process group.
pub fn all() -> Vec<ProcessRef> {
    PROCESSES.lock().clone()
}

/// The calling thread's process.
pub fn current() -> &'static Process {
    &crate::sched::current().process
}

/// The process with this pid (0 = the calling one).
pub fn lookup(pid: i32) -> Option<ProcessRef> {
    if pid == 0 {
        return Some(current().arc());
    }
    PROCESSES.lock().iter().find(|p| p.pid == pid).cloned()
}

/// The process in the process-table slot of `pid` (its low bits), whatever
/// its generation: what a pid truncated to fewer bits (a mutex's
/// `__owner`) still names.
pub fn by_slot(pid: i32) -> Option<ProcessRef> {
    let slot = pid & PID_MASK;
    PROCESSES.lock().iter().find(|p| p.pid & PID_MASK == slot).cloned()
}

/// The current thread hit a fault at EL0: end it (signals come later).
pub fn fault_current(reason: &str) -> ! {
    let t = crate::sched::current();
    crate::kprintln!(
        "process {}: thread {} terminated: {reason}",
        t.process.pid,
        t.tid
    );
    crate::sched::exit()
}

/// The channel with ID `chid`, if any.
fn channel_at(t: &[Option<Arc<Channel>>], chid: i32) -> Option<Arc<Channel>> {
    t.get(usize::try_from(chid).ok()?).cloned().flatten()
}

/// The lowest channel ID (from 1) that is neither a channel's nor a
/// message queue's.
fn free_chid(t: &[Option<Arc<Channel>>], queues: &alloc::collections::BTreeSet<i32>) -> usize {
    (1..)
        .find(|&i| t.get(i).is_none_or(|c| c.is_none()) && !queues.contains(&(i as i32)))
        .unwrap()
}

// ANCHOR: alloc_slot
/// Put `value` at the lowest free slot of `table` at or above `from` and
/// below `limit`, growing the table as needed; `None` if there is none.
/// The limit is what keeps a user-chosen index (a connection ID, a file
/// descriptor) from making the kernel allocate a table of that size.
fn alloc_slot<T>(table: &mut Vec<Option<T>>, from: usize, limit: usize, value: T) -> Option<usize> {
    let i = (from..table.len().min(limit))
        .find(|&i| table[i].is_none())
        .unwrap_or(table.len().max(from));
    if i >= limit {
        return None;
    }
    if table.len() <= i {
        table.resize_with(i + 1, || None);
    }
    table[i] = Some(value);
    Some(i)
}
// ANCHOR_END: alloc_slot

/// `RLIMIT_NOFILE` (`<sys/resource.h>`) and its default, the most file
/// descriptors a process may have (`sysconf(_SC_OPEN_MAX)`).
pub const RLIMIT_NOFILE: u32 = 5;
pub const DEFAULT_NOFILE: u64 = 1000;
/// File descriptors (connection IDs without `SIDE_CHANNEL`) are below
/// this whatever the limit: procnto answers EINVAL for an index at or
/// above it, EMFILE for one at or above `RLIMIT_NOFILE` (measured, IPC-1).
pub const FD_INDEX_LIMIT: u32 = 0xfffe;
/// Side channels a process may have at once: procnto's is
/// `RLIMIT_NOCONN_NP`, unlimited by default; this is the bound that keeps
/// the table of them from growing without end.
const SIDE_CHANNEL_LIMIT: usize = 1 << 20;

/// Supplementary groups a process may have (procnto's `NGROUPS_MAX`,
/// `sysconf(_SC_NGROUPS_MAX)`).
pub const NGROUPS_MAX: usize = 384;

/// A process's user and group IDs: real, effective and saved, and its
/// supplementary groups. Its effective user ID picks the domain, root
/// (0) or non-root, of its abilities (`ker::ability`).
#[derive(Clone)]
pub struct Cred {
    pub ruid: u32,
    pub euid: u32,
    pub suid: u32,
    pub rgid: u32,
    pub egid: u32,
    pub sgid: u32,
    pub groups: Vec<u32>,
}

impl Cred {
    pub const ROOT: Cred = Cred {
        ruid: 0,
        euid: 0,
        suid: 0,
        rgid: 0,
        egid: 0,
        sgid: 0,
        groups: Vec::new(),
    };

    /// `struct _cred_info` (`<sys/neutrino.h>`) with up to `ngroups`
    /// supplementary groups: ruid, euid, suid, rgid, egid, sgid, ngroups
    /// (the process's, even if fewer are listed), grouplist.
    pub fn info(&self, ngroups: usize) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [
            self.ruid, self.euid, self.suid, self.rgid, self.egid, self.sgid,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&(self.groups.len() as u32).to_le_bytes());
        for g in self.groups.iter().take(ngroups) {
            b.extend_from_slice(&g.to_le_bytes());
        }
        b
    }
}

impl Process {
    pub const fn new(pid: i32) -> Self {
        Self {
            pid,
            personality: AtomicU8::new(Personality::Native as u8),
            ppid: AtomicI32::new(KERNEL_PID),
            exit: IrqSpinLock::new(None),
            zombie: AtomicBool::new(false),
            loading: AtomicBool::new(false),
            nozombie: AtomicBool::new(false),
            copies: AtomicU32::new(0),
            execed: AtomicBool::new(false),
            unwaited: AtomicBool::new(false),
            // As procnto: the first processes start with no umask.
            umask: AtomicU32::new(0),
            sig_hint: AtomicBool::new(false),
            stop_hint: AtomicBool::new(false),
            cred: IrqSpinLock::new(Cred::ROOT),
            pgid: AtomicI32::new(0),
            sid: AtomicI32::new(0),
            threads: AtomicU32::new(0),
            ptids: IrqSpinLock::new(alloc::collections::BTreeSet::new()),
            pls: AtomicU64::new(0),
            start_time: AtomicU64::new(0),
            cpu_ns: AtomicU64::new(0),
            child_cpu_ns: AtomicU64::new(0),
            base_address: AtomicU64::new(0),
            initial_stack: AtomicU64::new(0),
            name: IrqSpinLock::new(String::new()),
            aspace: InitOnce::new(),
            mem: IrqSpinLock::new(MemMap::new()),
            limits: IrqSpinLock::new(BTreeMap::new()),
            abilities: IrqSpinLock::new(super::ability::Abilities::new()),
            timer_tolerance: AtomicU64::new(0),
            cpu_clock_offset: AtomicU64::new(0),
            signals: IrqSpinLock::new(Signals::new()),
            events: IrqSpinLock::new(Vec::new()),
            debug: IrqSpinLock::new(super::debug::DebugState::new()),
            cwd: IrqSpinLock::new(String::new()),
            root: IrqSpinLock::new(String::new()),
            channels: IrqSpinLock::new(Vec::new()),
            queue_ids: IrqSpinLock::new(alloc::collections::BTreeSet::new()),
            connections: IrqSpinLock::new(Vec::new()),
            side_connections: IrqSpinLock::new(Vec::new()),
            this: Weak::new(),
        }
    }

    /// A counted reference to this process, to keep.
    pub fn arc(&self) -> ProcessRef {
        if self.pid == KERNEL_PID {
            return ProcessRef::kernel();
        }
        ProcessRef(Some(self.this.upgrade().expect("process being freed")))
    }

    pub fn is_zombie(&self) -> bool {
        self.zombie.load(Ordering::Acquire)
    }

    /// (`si_code`, `si_status`) of a zombie.
    pub fn exit_status(&self) -> (i32, i32) {
        self.exit.lock().unwrap_or((super::signal::CLD_EXITED, 0))
    }

    pub fn personality(&self) -> Personality {
        match self.personality.load(Ordering::Relaxed) {
            1 => Personality::Qnx,
            _ => Personality::Native,
        }
    }

    pub fn set_personality(&self, p: Personality) {
        self.personality.store(p as u8, Ordering::Relaxed);
    }

    pub fn pgid(&self) -> i32 {
        match self.pgid.load(Ordering::Relaxed) {
            0 => self.pid,
            g => g,
        }
    }

    pub fn sid(&self) -> i32 {
        match self.sid.load(Ordering::Relaxed) {
            0 => self.pid,
            s => s,
        }
    }

    pub fn set_pgid(&self, pgid: i32) {
        self.pgid.store(pgid, Ordering::Relaxed);
    }

    /// Make the process the leader of a new session and process group.
    pub fn set_new_session(&self) {
        self.sid.store(self.pid, Ordering::Relaxed);
        self.pgid.store(self.pid, Ordering::Relaxed);
    }

    /// Put the process in session 1, process 1's, in a process group of its
    /// own (`procmgr_daemon()`).
    pub fn set_daemon_session(&self) {
        self.sid.store(KERNEL_PID, Ordering::Relaxed);
        self.pgid.store(self.pid, Ordering::Relaxed);
    }

    pub fn aspace(&self) -> &AddressSpace {
        self.aspace.get().expect("process without address space")
    }

    /// A QNX thread ID for a new thread: the lowest free one (as procnto).
    pub fn new_ptid(&self) -> u32 {
        let mut ids = self.ptids.lock();
        let id = (2..).find(|n| !ids.contains(n)).unwrap();
        ids.insert(id);
        id
    }

    /// Thread ID `ptid` is free again: its thread has been joined, or has
    /// ended detached.
    pub fn release_ptid(&self, ptid: u32) {
        self.ptids.lock().remove(&ptid);
    }

    /// Only the first thread's ID is in use (an exec()).
    pub fn reset_ptids(&self) {
        self.ptids.lock().clear();
    }

    pub fn thread_started(&self) {
        self.threads.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns true if that was the process's last thread.
    pub fn thread_exited(&self) -> bool {
        self.threads.fetch_sub(1, Ordering::AcqRel) == 1
    }

    /// Count the CPU time of a thread that ends.
    pub fn add_cpu_time(&self, ns: u64) {
        self.cpu_ns.fetch_add(ns, Ordering::Relaxed);
    }

    /// Nanoseconds its threads have run (its CPU-time clock).
    pub fn cpu_time(&self) -> u64 {
        let live: u64 = crate::sched::threads_of(self)
            .iter()
            .map(|t| t.cpu_time())
            .sum();
        self.cpu_ns.load(Ordering::Relaxed) + live
    }

    /// Nanoseconds its ended children (and theirs) ran.
    pub fn child_cpu_time(&self) -> u64 {
        self.child_cpu_ns.load(Ordering::Relaxed)
    }

    pub fn thread_count(&self) -> u32 {
        self.threads.load(Ordering::Acquire)
    }

    pub fn add_channel(&self, make: impl FnOnce(i32) -> Channel) -> i32 {
        let mut t = self.channels.lock();
        let chid = free_chid(&t, &self.queue_ids.lock());
        let ch = Arc::new(make(chid as i32));
        alloc_slot(&mut t, chid, usize::MAX, ch).expect("a free channel ID") as i32
    }

    /// The most file descriptors this process may have (`RLIMIT_NOFILE`,
    /// current), never above `FD_INDEX_LIMIT`.
    pub fn nofile(&self) -> u32 {
        self.limits
            .lock()
            .get(&RLIMIT_NOFILE)
            .map_or(DEFAULT_NOFILE, |l| l.0)
            .min(FD_INDEX_LIMIT as u64) as u32
    }

    /// Take the lowest free channel ID for a message queue it makes.
    pub fn add_queue_id(&self) -> i32 {
        let t = self.channels.lock();
        let mut q = self.queue_ids.lock();
        let id = free_chid(&t, &q) as i32;
        q.insert(id);
        id
    }

    /// Free message queue ID `id`; false if it isn't one.
    pub fn remove_queue_id(&self, id: i32) -> bool {
        let _t = self.channels.lock();
        self.queue_ids.lock().remove(&id)
    }

    pub fn channel(&self, chid: i32) -> Option<Arc<Channel>> {
        channel_at(&self.channels.lock(), chid)
    }

    /// Its connections to process `pid`'s channels: file descriptors, then
    /// side channel connections, by ID.
    pub fn connections_to(&self, pid: i32) -> Vec<i32> {
        let to = |t: &Vec<Option<Arc<Connection>>>, base: usize| -> Vec<i32> {
            t.iter()
                .enumerate()
                .filter(|(_, c)| c.as_ref().is_some_and(|c| c.channel.owner.pid == pid))
                .map(|(i, _)| (base + i) as i32)
                .collect()
        };
        let mut v = to(&self.connections.lock(), 0);
        v.extend(to(&self.side_connections.lock(), SIDE_CHANNEL as usize));
        v
    }

    /// Its channels, by ID.
    pub fn channel_list(&self) -> Vec<Arc<Channel>> {
        self.channels.lock().iter().flatten().cloned().collect()
    }

    /// Remove channel `chid` with its lanes (a lane's own ID isn't one).
    pub fn remove_channel(&self, chid: i32) -> Option<Arc<Channel>> {
        let mut t = self.channels.lock();
        let i = usize::try_from(chid).ok()?;
        let ch = t.get(i)?.as_ref().filter(|c| c.chid == chid)?.clone();
        for slot in t[i..].iter_mut() {
            match slot {
                Some(c) if Arc::ptr_eq(c, &ch) => *slot = None,
                _ => break,
            }
        }
        Some(ch)
    }

    /// Give channel `chid` `lanes` IDs in all, `chid` and the ones after
    /// it, all naming the channel. False if one of them is taken.
    pub fn widen_channel(&self, chid: i32, lanes: usize) -> bool {
        let mut t = self.channels.lock();
        let Some(ch) = channel_at(&t, chid) else {
            return false;
        };
        let i = chid as usize;
        let end = i + lanes;
        let queues = self.queue_ids.lock();
        let free = (i + 1..end).all(|j| match t.get(j) {
            Some(Some(c)) => Arc::ptr_eq(c, &ch),
            _ => !queues.contains(&(j as i32)),
        });
        drop(queues);
        if !free {
            return false;
        }
        if t.len() < end {
            t.resize_with(end, || None);
        }
        for slot in &mut t[i + 1..end] {
            *slot = Some(ch.clone());
        }
        true
    }

    /// Add a connection at the lowest free ID at or above `index`, below
    /// `RLIMIT_NOFILE` (`None` if there is none: EMFILE); or, if `index`
    /// has `SIDE_CHANNEL` set, at the lowest free side channel (the rest
    /// of the index is ignored, as procnto does: measured, IPC-1).
    pub fn add_connection(&self, index: u32, conn: Connection) -> Option<i32> {
        let conn = Arc::new(conn);
        if index & SIDE_CHANNEL != 0 {
            let i = alloc_slot(&mut self.side_connections.lock(), 0, SIDE_CHANNEL_LIMIT, conn)?;
            Some((SIDE_CHANNEL as usize + i) as i32)
        } else {
            let limit = self.nofile() as usize;
            Some(alloc_slot(&mut self.connections.lock(), index as usize, limit, conn)? as i32)
        }
    }

    fn connection_slot<R>(
        &self,
        coid: i32,
        f: impl FnOnce(&mut Option<Arc<Connection>>) -> R,
    ) -> Option<R> {
        let coid = u32::try_from(coid).ok()?;
        if coid & SIDE_CHANNEL != 0 {
            self.side_connections
                .lock()
                .get_mut((coid - SIDE_CHANNEL) as usize)
                .map(f)
        } else {
            self.connections.lock().get_mut(coid as usize).map(f)
        }
    }

    pub fn connection(&self, coid: i32) -> Option<Arc<Connection>> {
        self.connection_slot(coid, |s| s.clone()).flatten()
    }

    /// Every connection with server connection ID `scoid` to `server`:
    /// several when a QNX client shares a scoid among its connections to
    /// one channel (`ConnectAttach()` without `_NTO_COF_NOSHARE`).
    pub fn connections_by_scoid(&self, server: i32, scoid: i32) -> Vec<Arc<Connection>> {
        [&self.connections, &self.side_connections]
            .iter()
            .flat_map(|t| {
                t.lock()
                    .iter()
                    .flatten()
                    .filter(|c| c.scoid == scoid && c.channel.owner.pid == server)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The connections to servers other than procnto: (coid, server chid,
    /// scoid). After a fork the child must tell those servers (`_IO_DUP`).
    pub fn external_connections(&self) -> Vec<(i32, i32, i32)> {
        let mut out = Vec::new();
        for (table, base) in [
            (&self.connections, 0),
            (&self.side_connections, SIDE_CHANNEL),
        ] {
            for (i, c) in table.lock().iter().enumerate() {
                if let Some(c) = c
                    && c.channel.owner.pid != KERNEL_PID
                {
                    out.push(((base as usize + i) as i32, c.channel.chid, c.scoid));
                }
            }
        }
        out
    }

    /// Make file descriptor `to` refer to the same connection as `from`
    /// (replacing what `to` was); `dup2()`.
    pub fn dup_connection(&self, from: i32, to: i32) -> Option<()> {
        let mut t = self.connections.lock();
        let c = t.get(usize::try_from(from).ok()?)?.clone()?;
        let to = usize::try_from(to).ok().filter(|&to| to < self.nofile() as usize)?;
        if t.len() <= to {
            t.resize_with(to + 1, || None);
        }
        let old = t[to].replace(c);
        drop(t);
        // Dropped outside the lock: the last reference tells the server.
        drop(old);
        Some(())
    }

    /// Connect file descriptor `fd` (free) to process 1's channel, for a
    /// file process 1 opened for this process.
    pub fn connect_kernel(&self, fd: i32) -> Option<i32> {
        let sysmgr = KERNEL.channel(SYSMGR_CHID)?;
        self.add_connection(fd as u32, Connection::attach(sysmgr, self.pid))
    }

    /// The file descriptors (not side channels): (fd, close-on-exec).
    pub fn fds(&self) -> Vec<(i32, bool)> {
        self.connections
            .lock()
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let cloexec = c.as_ref()?.flags.load(Ordering::Relaxed) & channel::COF_CLOEXEC != 0;
                Some((i as i32, cloexec))
            })
            .collect()
    }

    /// Close the close-on-exec connections, as an exec does. Returns the
    /// connections closed, and those left open: a descriptor of another
    /// server that no other descriptor shares (not a side channel),
    /// which the server must hear of with `_IO_CLOSE` (as libc's `close()` sends it) or it
    /// keeps the open file forever; the new program's first thread
    /// sends it and closes them before it runs (procnto, measured,
    /// review PROCMGR-8: a `dup()`ed pair shares the connection, and
    /// the server hears nothing until the last descriptor goes).
    pub fn close_on_exec(&self) -> (Vec<i32>, Vec<i32>) {
        let mut closed = Vec::new();
        let mut deferred = Vec::new();
        for (table, base) in [
            (&self.connections, 0),
            (&self.side_connections, SIDE_CHANNEL),
        ] {
            for (i, slot) in table.lock().iter_mut().enumerate() {
                let Some(c) = slot.as_ref() else { continue };
                if c.flags.load(Ordering::Relaxed) & channel::COF_CLOEXEC == 0 {
                    continue;
                }
                let coid = (base as usize + i) as i32;
                // Descriptors only: a side-channel connection isn't a file,
                // and its server (a plain `MsgReceive()` loop) may never
                // answer an `_IO_CLOSE`, which would hang the exec.
                if base == 0 && c.channel.owner.pid != KERNEL_PID && Arc::strong_count(c) == 1 {
                    deferred.push(coid);
                } else {
                    *slot = None;
                    closed.push(coid);
                }
            }
        }
        (closed, deferred)
    }

    /// The connection `coid`, or the next one above it (after the file
    /// descriptors, the side channel connections).
    pub fn next_connection(&self, coid: i32) -> Option<(i32, Arc<Connection>)> {
        let coid = u32::try_from(coid).ok()?;
        let find = |table: &Vec<Option<Arc<Connection>>>, from: u32| {
            table
                .iter()
                .enumerate()
                .skip(from as usize)
                .find_map(|(i, c)| Some((i, c.as_ref()?.clone())))
        };
        // Past the last file descriptor connection: on to the side
        // channel ones (as procnto's `ker_connect_server_info`).
        if coid & SIDE_CHANNEL == 0
            && let Some((i, c)) = find(&self.connections.lock(), coid)
        {
            return Some((i as i32, c));
        }
        let from = if coid & SIDE_CHANNEL != 0 { coid - SIDE_CHANNEL } else { 0 };
        let (i, c) = find(&self.side_connections.lock(), from)?;
        Some(((SIDE_CHANNEL as usize + i) as i32, c))
    }

    pub fn remove_connection(&self, coid: i32) -> Option<Arc<Connection>> {
        self.connection_slot(coid, |s| s.take()).flatten()
    }
}

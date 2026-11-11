//! Process debugging: what the kernel does for a debugger (QNX's
//! `/proc/<pid>/as` and `ctl` devctls, see `proc::procfs`, used by
//! `pdebug`).
//!
//! A process stops in two ways, and runs only when neither holds:
//! - job control: `SIGSTOP` (and `POSIX_SPAWN_HOLD`, a process created
//!   stopped) until `SIGCONT`;
//! - its debugger: `DCMD_PROC_STOP`, or a point of interest it asked for
//!   (a fault in its fault set, a breakpoint, a single step, a signal in
//!   its trace set), until `DCMD_PROC_RUN`.
//!
//! Stopping takes effect as each thread next returns to EL0 (at the latest
//! at the next clock tick): it parks in STATE_STOPPED, its user registers
//! saved at the top of its kernel stack, where the debugger reads and
//! writes them. A point of interest records why (`_DEBUG_WHY_*`), what (the
//! fault or signal) and the thread, and delivers the debugger's event
//! (`DCMD_PROC_EVENT`) once per `DCMD_PROC_RUN` with `_DEBUG_RUN_ARM`.
//! A breakpoint or a step is reported with `_DEBUG_FLAG_TRACE_EXEC` or
//! `_DEBUG_FLAG_SSTEP`, the other points of interest with
//! `_DEBUG_FLAG_ISTOP`.
//!
//! Breakpoints are software ones: the instruction is replaced by
//! `brk #0` and restored on removal; reads of the process's memory show
//! the instruction, as in QNX (`hide_breaks`). Single steps use the AArch64
//! software step: `SPSR_EL1.SS` set in the stepping thread's saved state,
//! and `MDSCR_EL1.SS` set only on its way back to EL0 (`set_step`; with
//! `MDSCR_EL1.SS` set, a return to EL0 with `SPSR_EL1.SS` clear would
//! take a step exception at once), cleared at every context switch.
//!
//! Not yet: hardware breakpoints and watchpoints, stopping on thread,
//! fork or exec events, `SIGCHLD` with `CLD_STOPPED`.

use alloc::vec::Vec;
use core::sync::atomic::Ordering;

use super::event::{self, SigEvent};
use super::process::{self, Process};
use crate::arch::exception::TrapFrame;
use crate::sched::{self, State, Thread, ThreadRef};

// _DEBUG_FLAG_* (<sys/debug.h>)
pub const FLAG_STOPPED: u32 = 0x1;
pub const FLAG_ISTOP: u32 = 0x2;
pub const FLAG_IPINVAL: u32 = 0x10;
pub const FLAG_SSTEP: u32 = 0x40;
pub const FLAG_CURTID: u32 = 0x80;
pub const FLAG_TRACE_EXEC: u32 = 0x100;
pub const FLAG_RLC: u32 = 0x1_0000;
pub const FLAG_KLC: u32 = 0x2_0000;
/// The flags a debugger may change.
pub const FLAG_MASK: u32 = 0xf_0000;
// _DEBUG_WHY_*
pub const WHY_REQUESTED: u16 = 0;
pub const WHY_SIGNALLED: u16 = 1;
pub const WHY_FAULTED: u16 = 2;
pub const WHY_JOBCONTROL: u16 = 3;
pub const WHY_TERMINATED: u16 = 4;
// _DEBUG_RUN_*
const RUN_CLRSIG: u32 = 0x1;
const RUN_CLRFLT: u32 = 0x2;
const RUN_TRACE: u32 = 0x4;
const RUN_FAULT: u32 = 0x10;
const RUN_VADDR: u32 = 0x20;
const RUN_STEP: u32 = 0x40;
const RUN_STEP_ALL: u32 = 0x80;
const RUN_CURTID: u32 = 0x100;
const RUN_ARM: u32 = 0x200;
// Faults (<sys/fault.h>)
const FLTILL: u16 = 1;
const FLTBPT: u16 = 3;
const FLTTRACE: u16 = 4;
const FLTACCESS: u16 = 5;
const FLTBOUNDS: u16 = 6;
const FLTPAGE: u16 = 11;
/// `brk #0`
const BRK: u32 = 0xd420_0000;
/// `SPSR_EL1.SS`: step one instruction after the return to EL0.
const SPSR_SS: u64 = 1 << 21;
// Exception classes (ESR_EL1.EC)
const EC_UNKNOWN: u64 = 0x00;
const EC_IABT_LOW: u64 = 0x20;
const EC_PC_ALIGN: u64 = 0x22;
const EC_DABT_LOW: u64 = 0x24;
const EC_SP_ALIGN: u64 = 0x26;
const EC_SSTEP_LOW: u64 = 0x32;
const EC_BRK64: u64 = 0x3c;

/// A stop at a point of interest.
#[derive(Clone, Copy, Default)]
pub struct Stop {
    pub why: u16,
    pub what: u16,
    /// `_DEBUG_FLAG_ISTOP`, `_TRACE_EXEC` or `_SSTEP`.
    pub flags: u32,
    pub tid: u32,
    /// A fault's signal information (`si_signo`...), for `debug_thread_t`.
    pub info: Option<super::signal::SigInfo>,
}

/// A job control change its parent hasn't waited for yet
/// (`waitpid(WUNTRACED)`, `waitpid(WCONTINUED)`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum JobEvent {
    /// Stopped by this signal.
    Stopped(u16),
    Continued,
}

pub struct DebugState {
    /// Stopped by job control (`SIGSTOP`, `POSIX_SPAWN_HOLD`).
    pub job_stopped: bool,
    /// The last job control change, until the parent waits for it.
    pub job_event: Option<JobEvent>,
    /// Stopped by its debugger, until `DCMD_PROC_RUN`.
    pub held: bool,
    /// Its last thread is parked at its end until the debugger lets go
    /// (`park_at_exit`).
    pub exiting: bool,
    /// The debugger's process and the event to deliver at a stop.
    pub debugger: Option<(i32, SigEvent)>,
    /// Deliver the event at the next stop (`_DEBUG_RUN_ARM`).
    armed: bool,
    pub stop: Stop,
    /// Signals and faults that stop the process (bit n - 1 for n).
    trace: u64,
    faults: u64,
    /// The debugger's current thread (0: the first).
    pub cur_tid: u32,
    /// `_DEBUG_FLAG_RLC`, `_KLC`, ...
    pub flags: u32,
    /// Software breakpoints: (address, the instruction replaced).
    breaks: Vec<(u64, u32)>,
    /// A fault that stopped the process, to clear (`_DEBUG_RUN_CLRFLT`)
    /// before it runs on, or it ends the thread.
    pending_fault: bool,
    /// Traced signals the debugger passed on (ran without
    /// `_DEBUG_RUN_CLRSIG`): delivered, not traced again.
    passing: u64,
}

impl DebugState {
    pub const fn new() -> Self {
        Self {
            job_stopped: false,
            job_event: None,
            held: false,
            exiting: false,
            debugger: None,
            armed: false,
            stop: Stop {
                why: 0,
                what: 0,
                flags: 0,
                tid: 0,
                info: None,
            },
            trace: 0,
            faults: 0,
            cur_tid: 0,
            flags: 0,
            breaks: Vec::new(),
            pending_fault: false,
            passing: 0,
        }
    }

    fn stopped(&self) -> bool {
        self.job_stopped || self.held
    }
}

/// Set the CPU up for single steps of EL0 code: the OS lock released,
/// stepping off.
pub fn init_cpu() {
    // SAFETY: releases this CPU's debug OS lock; no effect on EL1
    // execution (MDSCR_EL1.KDE stays clear).
    unsafe { core::arch::asm!("msr oslar_el1, xzr", "isb", options(nostack)) };
    set_step(false);
}

/// Software step of EL0 code on this CPU (`MDSCR_EL1.SS`).
pub fn set_step(on: bool) {
    // SAFETY: only changes whether a return to EL0 steps.
    unsafe {
        // Written only when it changes: the write and its `isb` cost on
        // every context switch.
        core::arch::asm!(
            "mrs {t}, mdscr_el1",
            "bic {u}, {t}, #1",
            "orr {u}, {u}, {on}",
            "cmp {u}, {t}",
            "b.eq 2f",
            "msr mdscr_el1, {u}",
            "isb",
            "2:",
            t = out(reg) _,
            u = out(reg) _,
            on = in(reg) on as u64,
            options(nostack)
        );
    }
}

/// The last thing before a return to EL0: step if the thread is to.
pub fn before_eret(frame: &TrapFrame) {
    set_step(frame.spsr & SPSR_SS != 0);
}

/// Must the current thread stop before returning to EL0? A job control
/// stop not if the process is being killed; a stop by its debugger even
/// then: as in procnto, SIGKILL ends a process its debugger holds only
/// once the debugger lets it go (runs it on or closes it), sending the
/// debugger no event.
fn must_stop(p: &Process) -> bool {
    if !p.stop_hint.load(Ordering::Acquire) {
        return false;
    }
    // A process's locks nest `debug` before `signals` (as in `run`); none
    // is held across `killed` here, which needs only the snapshot.
    let (job, held) = {
        let st = p.debug.lock();
        (st.job_stopped, st.held)
    };
    held || job && !killed(p)
}

const SIGKILL: usize = 9;

// ANCHOR: park_if_stopped
/// On the way back to EL0: park while the process is stopped or the
/// thread held. Returns at once if neither.
pub fn park_if_stopped(me: &Thread) {
    loop {
        let held = me.held.load(Ordering::Acquire) && !killed(&me.process);
        if !(held || must_stop(&me.process)) || me.exit_on_return.load(Ordering::Relaxed) {
            break;
        }
        let st = me.process.debug.lock();
        if !st.stopped() && !me.held.load(Ordering::Acquire) {
            break;
        }
        // Blocked under the lock that `release` (or `thread_continue`)
        // takes to wake us.
        sched::prepare_block(State::Stopped);
        drop(st);
        sched::block();
    }
}
// ANCHOR_END: park_if_stopped

/// Is SIGKILL pending for `p` (its stopped and held threads go on, to
/// end)?
fn killed(p: &Process) -> bool {
    p.signals.lock().pending & (1 << (SIGKILL - 1)) != 0
}

/// Hold thread `t` (`_NTO_TCTL_*_HOLD`): it stops before it next returns
/// to EL0. Holds don't count: one continue undoes any number.
pub fn thread_hold(t: &Thread) {
    t.held.store(true, Ordering::Release);
    sched::interrupt(t);
}

/// Continue a held thread `t`, unless its process is stopped (it goes on
/// with the process then).
pub fn thread_continue(t: &Thread) {
    if !t.held.swap(false, Ordering::AcqRel) {
        return;
    }
    let st = t.process.debug.lock();
    if !st.stopped() && t.state() == State::Stopped {
        sched::wake(t);
    }
}

/// Wake the parked threads of `p` if it is to run.
fn wake_parked(p: &Process) {
    for t in sched::threads_of(p) {
        if t.state() == State::Stopped {
            sched::wake(&t);
        }
    }
}

/// Job control: `SIGSTOP` (or a held spawn), or SIGTSTP, SIGTTIN or
/// SIGTTOU at its default action, stops `p` (`signo` the stop signal);
/// `SIGCONT` lets it go on (unless its debugger holds it). A debugger that
/// traces the signal sees it instead, as a stop at a point of interest
/// (that is how it interrupts a running process).
pub fn job_stop(p: &Process, notify: bool, signo: u16) {
    let mut st = p.debug.lock();
    let qnx_signo = crate::personality::qnx::signo_out(signo as usize).unwrap_or(0) as u16;
    if st.debugger.is_some() && qnx_signo != 0 && st.trace & (1 << (qnx_signo - 1)) != 0 {
        let tid = cur_thread(p, &st).map_or(1, |t| t.ptid);
        let stop = Stop {
            why: WHY_SIGNALLED,
            what: qnx_signo,
            flags: FLAG_ISTOP,
            tid,
            info: Some(super::signal::SigInfo {
                signo: signo as i32,
                ..Default::default()
            }),
        };
        stop_at(p, &mut st, stop);
        return;
    }
    p.stop_hint.store(true, Ordering::Release);
    let was = core::mem::replace(&mut st.job_stopped, true);
    if was || !notify {
        return;
    }
    st.job_event = Some(JobEvent::Stopped(signo));
    drop(st);
    job_changed(p, true);
}

pub fn job_continue(p: &Process) {
    let mut st = p.debug.lock();
    let was = core::mem::replace(&mut st.job_stopped, false);
    if was {
        st.job_event = Some(JobEvent::Continued);
    }
    if !st.stopped() {
        drop(st);
        wake_parked(p);
    }
    if was {
        job_changed(p, false);
    }
}

/// A job control change of `p`: as procnto, a stop sends its parent
/// SIGCHLD (`si_code` 0, `si_uid` -1; not with `SA_NOCLDSTOP`), a
/// continuation nothing; process 1 answers the waits it settles.
fn job_changed(p: &Process, stopped: bool) {
    const SA_NOCLDSTOP: u32 = 0x8;
    let ppid = p.ppid.load(Ordering::Relaxed);
    if stopped
        && ppid != process::KERNEL_PID
        && let Some(parent) = process::lookup(ppid)
        && parent.signals.lock().actions[super::signal::SIGCHLD].flags & SA_NOCLDSTOP == 0
    {
        let info = super::signal::SigInfo {
            code: 0,
            pid: p.pid,
            uid: -1,
            ..Default::default()
        };
        super::signal::post(&parent, super::signal::SIGCHLD, info);
    }
    // Straight to process 1's channel, as a process's end is: a stop sent
    // from the tick (a timer's SIGSTOP) or by process 1's own threads (job
    // control, POSIX_SPAWN_HOLD) comes from no process with a side
    // connection, and the pulse was lost (review IPC-15).
    if let Some(sysmgr) = process::KERNEL.channel(process::SYSMGR_CHID) {
        let prio = crate::sched::current().base_priority();
        super::msg::channel_pulse(&sysmgr, prio, process::PULSE_JOB_CHANGED, p.pid as u64, 0);
    }
}

/// Stop at a point of interest: record it and tell the debugger.
fn stop_at(p: &Process, st: &mut DebugState, stop: Stop) {
    p.stop_hint.store(true, Ordering::Release);
    st.held = true;
    st.stop = stop;
    if st.armed
        && let Some((pid, ev)) = st.debugger
        && let Some(dp) = process::lookup(pid)
    {
        st.armed = false;
        let _ = event::deliver(&dp, None, &ev, 0);
    }
}

/// `DCMD_PROC_STOP`: the debugger stops `p`.
pub fn request_stop(p: &Process) {
    let mut st = p.debug.lock();
    let tid = cur_thread(p, &st).map_or(1, |t| t.ptid);
    let stop = Stop {
        why: WHY_REQUESTED,
        what: 0,
        flags: FLAG_ISTOP,
        tid,
        info: None,
    };
    stop_at(p, &mut st, stop);
}

/// The last thread `me` of a debugged process ends: as procnto
/// (`nto_debug_process_exit`), it parks, stopped at
/// `_DEBUG_WHY_TERMINATED` (what: the exit status, 0 for a signal), and
/// the debugger hears of it if armed. Neither SIGKILL nor a run ends it:
/// only the debugger letting go does, and then, the loader context's
/// siginfo cleared, its parent sees it killed by signal 0 (SIGCHLD's
/// `si_code` `CLD_KILLED`, a wait status of 0).
pub fn park_at_exit(me: &Thread) {
    let p = &me.process;
    let (code, status) = p.exit.lock().unwrap_or((super::signal::CLD_EXITED, 0));
    let mut st = p.debug.lock();
    if st.debugger.is_none() {
        return;
    }
    let stop = Stop {
        why: WHY_TERMINATED,
        what: if code == super::signal::CLD_EXITED { status as u16 } else { 0 },
        flags: FLAG_ISTOP,
        tid: me.ptid,
        info: None,
    };
    stop_at(p, &mut st, stop);
    st.exiting = true;
    while st.debugger.is_some() {
        // Blocked under the lock that `detach` takes to wake us.
        sched::prepare_block(State::Stopped);
        drop(st);
        sched::block();
        st = p.debug.lock();
    }
    st.exiting = false;
    drop(st);
    *p.exit.lock() = Some((super::signal::CLD_KILLED, 0));
}

/// The debugger's current thread of `p` (`DCMD_PROC_CURTHREAD`), else the
/// first one.
pub fn cur_thread(p: &Process, st: &DebugState) -> Option<ThreadRef> {
    let mut threads = sched::threads_of(p);
    threads.sort_by_key(|t| t.ptid);
    threads
        .iter()
        .cloned()
        .find(|t| t.ptid == st.cur_tid)
        .or_else(|| threads.first().cloned())
}

/// `DCMD_PROC_EVENT`: deliver `ev` to process `debugger` at stops. A new
/// debugger starts with `_DEBUG_FLAG_RLC` (run on last close), as in
/// procnto.
pub fn set_event(p: &Process, debugger: i32, ev: SigEvent) {
    let mut st = p.debug.lock();
    if st.debugger.is_none() {
        st.flags |= FLAG_RLC;
    }
    st.debugger = Some((debugger, ev));
}

/// The debugger let go of `p` (its last `as`/`ctl` closed): killed with
/// `_DEBUG_FLAG_KLC`, else run on, its breakpoints removed.
pub fn detach(p: &Process) {
    let mut st = p.debug.lock();
    if st.debugger.is_none() {
        return;
    }
    let kill = st.flags & FLAG_KLC != 0;
    let breaks = core::mem::take(&mut st.breaks);
    st.debugger = None;
    st.held = false;
    st.flags = 0;
    st.trace = 0;
    st.faults = 0;
    st.pending_fault = false;
    drop(st);
    for (addr, insn) in breaks {
        let _ = write_insn(p, addr, insn);
    }
    if kill {
        super::signal::post(p, SIGKILL, super::signal::SigInfo::default());
    }
    wake_parked(p);
}

/// `struct _debug_run64` (`DCMD_PROC_RUN`).
pub struct Run {
    pub flags: u32,
    pub tid: u32,
    pub trace: u64,
    pub fault: u64,
    pub ip: u64,
}

/// `DCMD_PROC_RUN`: let `p` run on, with what the debugger asks.
pub fn run(p: &Process, r: &Run) -> Result<(), u32> {
    let mut st = p.debug.lock();
    if r.flags & RUN_CURTID != 0 {
        st.cur_tid = r.tid;
    }
    // A process parked at its end doesn't run on: as procnto, the stop
    // stays, no longer a point of interest.
    if st.exiting {
        st.armed = r.flags & RUN_ARM != 0;
        st.stop.flags &= !FLAG_ISTOP;
        return Ok(());
    }
    if r.flags & RUN_TRACE != 0 {
        st.trace = r.trace;
    }
    if r.flags & RUN_FAULT != 0 {
        st.faults = r.fault;
    }
    if r.flags & RUN_CLRFLT != 0 {
        st.pending_fault = false;
    }
    if st.stop.why == WHY_SIGNALLED {
        let signo = st.stop.what as usize;
        if (1..=64).contains(&signo) {
            if r.flags & RUN_CLRSIG != 0 {
                p.signals.lock().discard(crate::personality::qnx::signo_in(signo));
            } else {
                st.passing |= 1 << (signo - 1);
            }
        }
    }
    let t = cur_thread(p, &st).ok_or(super::errno::ESRCH)?;
    if r.flags & (RUN_VADDR | RUN_STEP | RUN_STEP_ALL) != 0 {
        if t.state() != State::Stopped {
            return Err(super::errno::EBUSY);
        }
        // SAFETY: `t` is parked: its saved user state doesn't change.
        unsafe {
            t.with_user_frame(|f| {
                if r.flags & RUN_VADDR != 0 {
                    f.elr = r.ip;
                }
                if r.flags & (RUN_STEP | RUN_STEP_ALL) != 0 {
                    f.spsr |= SPSR_SS;
                }
            })
        };
    }
    st.armed = r.flags & RUN_ARM != 0;
    st.held = false;
    st.stop = Stop::default();
    let go = !st.stopped();
    drop(st);
    if go {
        wake_parked(p);
    }
    Ok(())
}

/// Write instruction `insn` at `addr` in `p`, whatever the page's
/// protection, and make it visible to instruction fetches.
fn write_insn(p: &Process, addr: u64, insn: u32) -> Result<u32, u32> {
    let mut old = [0u8; 4];
    write_mem(p, addr, &insn.to_le_bytes(), Some(&mut old))?;
    Ok(u32::from_le_bytes(old))
}

/// Write `data` at `addr` in `p`'s memory through its frames (code pages
/// included), returning the old bytes in `old`.
pub fn write_mem(
    p: &Process,
    addr: u64,
    data: &[u8],
    mut old: Option<&mut [u8]>,
) -> Result<(), u32> {
    use crate::vm::pmem::PAGE_SIZE;
    let mut done = 0;
    while done < data.len() {
        let va = addr.checked_add(done as u64).ok_or(super::errno::EFAULT)?;
        let page = va & !(PAGE_SIZE - 1);
        let off = (va - page) as usize;
        let n = (PAGE_SIZE as usize - off).min(data.len() - done);
        crate::vm::map::own_page(p, page)?;
        let (pa, prot) = p.aspace().page(page).ok_or(super::errno::EFAULT)?;
        crate::vm::kmem::with_frame(pa, |f| {
            if let Some(o) = old.as_deref_mut() {
                o[done..done + n].copy_from_slice(&f[off..off + n]);
            }
            f[off..off + n].copy_from_slice(&data[done..done + n]);
            if prot.exec {
                crate::arch::mmu::sync_icache(f.as_ptr() as u64 + off as u64, n as u64);
            }
        });
        done += n;
    }
    Ok(())
}

/// Memory of `p` at `addr` as read (`buf`) shows the instructions its
/// breakpoints replaced, not the breakpoints (debuggers read code and
/// would take a `brk` there for one of the program's own).
pub fn hide_breaks(p: &Process, addr: u64, buf: &mut [u8]) {
    let end = addr.saturating_add(buf.len() as u64);
    for &(at, insn) in &p.debug.lock().breaks {
        for (i, b) in insn.to_le_bytes().iter().enumerate() {
            let a = at + i as u64;
            if (addr..end).contains(&a) {
                buf[(a - addr) as usize] = *b;
            }
        }
    }
}

/// `DCMD_PROC_BREAK`: `size` -1 removes the breakpoint at `addr`, else
/// sets an execution one (software). Returns the breakpoint's type.
pub fn set_break(p: &Process, kind: u16, size: i16, addr: u64) -> Result<u16, u32> {
    const BREAK_EXEC: u16 = 0x1;
    const BREAK_MASK: u16 = 0xf;
    const BREAK_SOFT: u16 = 0x8000;
    let mut st = p.debug.lock();
    let at = st.breaks.iter().position(|(a, _)| *a == addr);
    if size == -1 {
        let (_, insn) = st.breaks.remove(at.ok_or(super::errno::ESRCH)?);
        write_insn(p, addr, insn)?;
        return Ok(kind);
    }
    if kind & BREAK_MASK != BREAK_EXEC || !addr.is_multiple_of(4) {
        return Err(super::errno::EINVAL);
    }
    if at.is_none() {
        let insn = write_insn(p, addr, BRK)?;
        st.breaks.push((addr, insn));
    }
    Ok(kind | BREAK_SOFT)
}

/// The signal a fault amounts to (`si_signo`, `si_code`), as QNX's
/// kernel gives it.
fn fault_signal(flt: u16) -> (i32, i32) {
    const SIGILL: i32 = 4;
    const SIGTRAP: i32 = 5;
    const SIGBUS: i32 = 10;
    const SIGSEGV: i32 = 11;
    match flt {
        FLTILL => (SIGILL, 1),     // ILL_ILLOPC
        FLTBPT => (SIGTRAP, 1),    // TRAP_BRKPT
        FLTTRACE => (SIGTRAP, 2),  // TRAP_TRACE
        FLTBOUNDS => (SIGBUS, 1),  // BUS_ADRALN
        FLTACCESS => (SIGSEGV, 2), // SEGV_ACCERR
        _ => (SIGSEGV, 1),         // SEGV_MAPERR
    }
}

/// The signal information of fault `flt` at `frame`.
fn fault_info(flt: u16, frame: &TrapFrame) -> super::signal::SigInfo {
    // A misaligned data access (`_NTO_TCTL_ALIGN_FAULT`): an access fault
    // that procnto signals as SIGBUS, BUS_ADRALN.
    let (signo, code) = if is_align_fault(frame) { (10, 1) } else { fault_signal(flt) };
    let addr = match frame.esr >> 26 {
        EC_IABT_LOW | EC_DABT_LOW | EC_SP_ALIGN => frame.far,
        _ => frame.elr,
    };
    super::signal::SigInfo {
        signo,
        code,
        fault: Some((flt as i32, frame.elr, addr)),
        ..Default::default()
    }
}

/// An exception from EL0 that no debugger takes: its signal, if it is a
/// fault (else false: an exception the kernel can't make sense of).
/// Will the fault in `frame` go to a signal handler of the thread (a
/// program probing, e.g. OpenSSL trying CPU features under SIGILL)?
pub fn fault_caught(frame: &TrapFrame) -> bool {
    fault_of(frame).is_some_and(|flt| super::signal::caught(fault_info(flt, frame).signo as usize))
}

pub fn fault_to_signal(frame: &TrapFrame) -> bool {
    match fault_of(frame) {
        Some(flt) => {
            super::signal::fault(fault_info(flt, frame));
            true
        }
        None => false,
    }
}

/// A data abort from EL0 for a misaligned access (DFSC 0x21).
fn is_align_fault(frame: &TrapFrame) -> bool {
    frame.esr >> 26 == EC_DABT_LOW && frame.esr & 0x3f == 0x21
}

/// The fault number of an exception from EL0 (`<sys/fault.h>`), if any.
fn fault_of(frame: &TrapFrame) -> Option<u16> {
    let ec = frame.esr >> 26;
    Some(match ec {
        EC_BRK64 => FLTBPT,
        EC_SSTEP_LOW => FLTTRACE,
        EC_UNKNOWN => FLTILL,
        EC_PC_ALIGN | EC_SP_ALIGN => FLTBOUNDS,
        EC_IABT_LOW | EC_DABT_LOW => {
            // DFSC/IFSC: translation faults are page faults, the others
            // (permission, access flag) access faults.
            if frame.esr & 0x3c == 0x04 {
                FLTPAGE
            } else {
                FLTACCESS
            }
        }
        _ => return None,
    })
}

// ANCHOR: debug_fault
/// An exception from EL0 that may interest the current process's
/// debugger: if it does, the process stops (the thread parks on its way
/// back to EL0) and this returns true; else the fault takes its course.
pub fn fault(frame: &mut TrapFrame) -> bool {
    let me = sched::current();
    let p = &*me.process;
    let Some(flt) = fault_of(frame) else {
        return false;
    };
    if flt == FLTTRACE {
        set_step(false);
    }
    let mut st = p.debug.lock();
    if st.debugger.is_none() {
        // A step nobody asked for (stepping left on): just go on.
        if flt == FLTTRACE {
            frame.spsr &= !SPSR_SS;
            return true;
        }
        return false;
    }
    let flags = match flt {
        FLTTRACE => {
            frame.spsr &= !SPSR_SS;
            FLAG_SSTEP
        }
        FLTBPT if st.breaks.iter().any(|(a, _)| *a == frame.elr) => FLAG_TRACE_EXEC,
        _ if st.faults & (1 << (flt - 1)) != 0 => {
            st.pending_fault = flt != FLTBPT;
            FLAG_ISTOP
        }
        _ => return false,
    };
    let stop = Stop {
        why: WHY_FAULTED,
        what: flt,
        flags,
        tid: me.ptid,
        info: Some(fault_info(flt, frame)),
    };
    st.cur_tid = me.ptid;
    stop_at(p, &mut st, stop);
    true
}
// ANCHOR_END: debug_fault

/// After a stop at a fault: a fault the debugger didn't clear is still
/// fatal.
pub fn fault_cleared(p: &Process) -> bool {
    !p.debug.lock().pending_fault
}

/// A signal for `p`: stops it if its debugger traces the signal
/// (`_DEBUG_WHY_SIGNALLED`). Returns whether it did.
pub fn trace_signal(p: &Process, signo: usize) -> bool {
    let me = sched::current();
    let mut st = p.debug.lock();
    // A debugger speaks QNX's numbers (`_DEBUG_*`).
    let Some(qnx_signo) = crate::personality::qnx::signo_out(signo) else {
        return false;
    };
    let bit = 1 << (qnx_signo - 1);
    if st.debugger.is_none() || st.trace & bit == 0 {
        return false;
    }
    // Passed on by the debugger at its last run: delivered now.
    if st.passing & bit != 0 {
        st.passing &= !bit;
        return false;
    }
    let stop = Stop {
        why: WHY_SIGNALLED,
        what: signo as u16,
        flags: FLAG_ISTOP,
        tid: me.ptid,
        info: Some(super::signal::SigInfo {
            signo: signo as i32,
            ..Default::default()
        }),
    };
    stop_at(p, &mut st, stop);
    true
}

/// What `debug_thread_t` says of a thread beyond its scheduling.
pub struct ThreadView {
    pub flags: u32,
    pub why: u16,
    pub what: u16,
    pub ip: u64,
    pub sp: u64,
    pub tls: u64,
    /// Why it stopped, as signal information (`info`).
    pub info: Option<super::signal::SigInfo>,
}

/// What `debug_thread_t` says of thread `t` of `p`.
pub fn thread_view(p: &Process, t: &Thread) -> ThreadView {
    let st = p.debug.lock();
    let mut flags = 0;
    let (mut why, mut what, mut info) = (0, 0, None);
    if t.state() == State::Stopped {
        flags |= FLAG_STOPPED;
    }
    if st.stopped() && st.stop.tid == t.ptid {
        flags |= st.stop.flags;
        (why, what, info) = (st.stop.why, st.stop.what, st.stop.info);
    } else if st.job_stopped {
        why = WHY_JOBCONTROL;
    }
    if cur_thread(p, &st).is_some_and(|c| core::ptr::eq(&*c, t)) {
        flags |= FLAG_CURTID;
    }
    flags |= st.flags & FLAG_MASK;
    drop(st);
    let tls = t.tls.load(Ordering::Relaxed);
    // SAFETY: only read while `t` is blocked (checked by user_state).
    let (ip, sp) = match unsafe { t.user_state() } {
        Some((frame, _, _)) => (frame.elr, frame.sp_el0),
        None => {
            flags |= FLAG_IPINVAL;
            (0, 0)
        }
    };
    ThreadView {
        flags,
        why,
        what,
        ip,
        sp,
        tls,
        info,
    }
}

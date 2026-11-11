//! Kernel calls (QNX `ker/`).
//!
//! QNX 8.0 AArch64 convention, as used by the SDP's libc stubs: number in
//! w8, arguments in x0..x7, `svc #0x51`, result in x0. A failing call
//! resumes one instruction after the normal return point, with the errno
//! value in x0; the stubs put the error handling there (`b __ker_err`, or
//! `neg x0, x0; ret` for the `_r` variants) and a `ret` at the normal point.

use core::sync::atomic::Ordering;

use crate::arch::exception::TrapFrame;
use crate::sched;

pub mod ability;
pub mod channel;
pub mod clock;
pub mod debug;
pub mod dupfd;
pub mod entropy;
pub mod errno;
pub mod event;
pub mod interrupt;
pub mod kpipe;
pub mod mqueue;
pub mod msg;
pub mod number;
pub mod offline;
pub mod process;
pub mod servmon;
pub mod signal;
pub mod sync;
pub mod thread;
pub mod timer;
pub mod trace;
pub mod umem;

/// A kernel call's result: a value, or an errno.
pub type KerResult = Result<u64, u32>;

/// Handle a kernel call trapped in `frame`.
pub fn dispatch(frame: &mut TrapFrame) {
    use number::*;
    crate::percpu::this()
        .kercalls
        .fetch_add(1, Ordering::Relaxed);
    // A cancelled thread at a cancellation point goes to its cancel stub,
    // without making the call, or once it fails with EINTR (woken by the
    // cancel, or a signal).
    let point = is_cancellation_point(frame.kercall_number(), frame);
    if point && thread::cancellation_point(frame) {
        // The call isn't made, but it was the call a TimerTimeout() was
        // for: the cleanup handlers' blocking calls don't inherit it
        // (procnto, measured).
        clear_timeout();
        user_return(frame);
        return;
    }
    let a = |i| frame.kercall_arg(i);
    if frame.kercall_number() == __KER_SIGNAL_RETURN {
        // Restores the whole user state: not completed like other calls.
        if let Err(e) = signal::signal_return(frame, a(0)) {
            frame.complete_kercall(Err(sched::current().process.personality().errno_out(e)));
        }
        signal::deliver(frame);
        return;
    }
    let num = frame.kercall_number();
    trace_enter(num, frame);
    // Whether the caller had interrupts enabled (SPSR.I clear): then a long
    // operation in the call may let a higher-priority thread run
    // (`sched::preemption_point`). Restored after: a kernel call made from
    // within one keeps the outer one's.
    let outer = sched::current()
        .kcall_preemptible
        .swap(frame.spsr & (1 << 7) == 0, Ordering::Relaxed);
    let result = match num {
        __KER_NOP => Ok(0),
        __KER_CACHE_CONTROL => cache_control(a(0), a(1), a(2) as u32),
        __KER_TRACE_EVENT => trace::trace_event(a(0)),
        // The server thread already runs at the client's priority (if it
        // was raised by the message).
        __KER_MSG_CURRENT => msg::msg_current(a(0) as i64),
        __KER_SYS_CPUPAGE_GET => sys_cpupage_get(a(0) as i32),
        __KER_SYS_CPUPAGE_SET => sys_cpupage_set(a(0) as i32, a(1)),
        __KER_MSG_SENDV | __KER_MSG_SENDVNC => {
            msg::msg_sendv(a(0) as i32, a(1), a(2) as i64, a(3), a(4) as i64)
        }
        __KER_MSG_RECEIVEV => msg::msg_receivev(a(0) as i32, a(1), a(2) as i64, a(3)),
        __KER_MSG_RECEIVEPULSEV => msg::msg_receive_pulsev(a(0) as i32, a(1), a(2) as i64, a(3)),
        __KER_OSCRAFT_MSG_FORWARDV => msg::msg_forwardv(a(0) as i64, a(1), a(2) as i64, a(3), a(4) as i64, a(5)),
        __KER_MSG_REPLYV => msg::msg_replyv(a(0) as i64, a(1), a(2), a(3) as i64),
        __KER_MSG_INFO => msg::msg_info_call(a(0) as i64, a(1)),
        __KER_MSG_READV => msg::msg_readv(a(0) as i64, a(1), a(2) as i64, a(3)),
        __KER_MSG_WRITEV => msg::msg_writev(a(0) as i64, a(1), a(2) as i64, a(3)),
        __KER_MSG_ERROR => {
            let error = sched::current().process.personality().errno_in(a(1) as u32);
            msg::msg_error(a(0) as i64, error)
        }
        __KER_MSG_SEND_PULSE => msg::msg_send_pulse(a(0) as i32, a(1) as i32, a(2) as i32, a(3)),
        // The same, with a pointer-sized value (`MsgSendPulsePtr()`).
        __KER_MSG_SEND_PULSEPTR => msg::msg_send_pulse(a(0) as i32, a(1) as i32, a(2) as i32, a(3)),
        __KER_MSG_DELIVER_EVENT => event::msg_deliver_event(a(0) as i64, a(1)),
        __KER_MSG_REGISTER_EVENT => event::msg_register_event(a(0), a(1) as i32, a(2)),
        __KER_MSG_VERIFY_EVENT => event::msg_verify_event(a(0) as i64, a(1)),
        // QNX 8: "any deadlock error that message pausing is meant to
        // handle cannot happen in this release": not supported.
        __KER_MSG_PAUSE => Err(errno::ENOSYS),
        __KER_SIGNAL_KILL => signal::signal_kill(
            a(0) as u32,
            a(1) as i32,
            a(2) as i32,
            a(3) as i32,
            a(4) as i32,
            a(5),
        ),
        __KER_SIGNAL_KILL_SIGVAL => signal::signal_kill_sigval(
            a(0) as u32,
            a(1) as i32,
            a(2) as i32,
            a(3) as i32,
            a(4) as i32,
            a(5),
        ),
        __KER_SIGNAL_SUSPEND => signal::signal_suspend(a(0)),
        __KER_SIGNAL_WAITINFO => signal::signal_waitinfo(a(0), a(1), a(2)),
        __KER_SIGNAL_ACTION => signal::signal_action(a(0) as i32, a(1), a(2) as i32, a(3), a(4)),
        __KER_SIGNAL_PROCMASK => {
            signal::signal_procmask(a(0) as i32, a(1) as i32, a(2) as u32, a(3), a(4))
        }
        __KER_CONNECT_SERVER_INFO => channel::connect_server_info(a(0) as i32, a(1) as i32, a(2)),
        __KER_CONNECT_FLAGS => {
            channel::connect_flags(a(0) as i32, a(1) as i32, a(2) as u32, a(3) as u32)
        }
        __KER_CONNECT_CLIENT_INFO => {
            channel::connect_client_info(a(0) as i32, a(1), a(2) as i32, a(3), a(4) as i32, a(5))
        }
        __KER_CHANNEL_CREATE => channel::channel_create(a(0) as u32),
        __KER_CHANNEL_DESTROY => channel::channel_destroy(a(0) as i32),
        __KER_SYS_SRANDOM => entropy::sys_srandom(a(0), a(1), a(2)),
        __KER_CONNECT_DUPFD => dupfd::connect_dupfd(a(0) as u32, a(1), a(2), a(3), a(4)),
        __KER_CHANNEL_CTL => channel::channel_ctl(a(0) as i32, a(1) as u32, a(2)),
        __KER_CONNECT_ATTACH => channel::connect_attach(
            a(0) as u32,
            a(1) as i32,
            a(2) as i32,
            a(3) as u32,
            a(4) as u32,
        ),
        __KER_CONNECT_DETACH => channel::connect_detach(a(0) as i32),
        __KER_THREAD_CREATE => thread::thread_create(a(0) as i32, a(1), a(2), a(3)),
        __KER_THREAD_DESTROY => thread_destroy(a(0) as i32, a(2)),
        __KER_THREAD_DETACH => thread::thread_detach(a(0) as u32),
        __KER_THREAD_JOIN => thread::thread_join(a(0) as u32, a(1)),
        __KER_THREAD_CTL => thread::thread_ctl(0, 0, a(0) as u32, a(1)),
        __KER_THREAD_CTLEXT => thread::thread_ctl(a(0) as i32, a(1) as u32, a(2) as u32, a(3)),
        __KER_CLOCK_TIME => clock::clock_time(a(0) as u32, a(1), a(2)),
        __KER_CLOCK_ID => clock::clock_id(a(0) as i32, a(1) as u32),
        __KER_CLOCK_ADJUST => clock::clock_adjust(a(0) as u32, a(1), a(2)),
        __KER_CLOCK_PERIOD => clock::clock_period(a(0) as u32, a(1), a(2)),
        __KER_TIMER_CREATE => timer::timer_create(a(0) as u32, a(1)),
        __KER_TIMER_DESTROY => timer::timer_destroy(a(0) as i32),
        __KER_TIMER_SETTIME => timer::timer_settime(a(0) as i32, a(1) as u32, a(2), a(3)),
        __KER_TIMER_INFO => timer::timer_info(a(0) as i32, a(1) as i32, a(2) as u32, a(3)),
        __KER_TIMER_ALARM => timer::timer_alarm(a(0) as u32, a(1), a(2)),
        __KER_TIMER_TIMEOUT => timer::timer_timeout(a(0) as u32, a(1) as u32, a(2), a(3), a(4)),
        __KER_SYNC_DESTROY => sync::sync_destroy(a(0)),
        __KER_SYNC_RWLOCK => sync::rwlock(a(0), a(1) as u32),
        __KER_SYNC_SEM_POST => sync::sem_post(a(0)),
        __KER_SYNC_SEM_WAIT => sync::sem_wait(a(0), a(1) != 0),
        __KER_SYNC_BARRIER_WAIT => sync::barrier_wait(a(0)),
        __KER_SYNC_CTL => sync::sync_ctl(a(0) as u32, a(1), a(2)),
        __KER_SYNC_TYPE_DESTROY => sync::sync_type_destroy(a(0) as u32, a(1)),
        // OsCraft's calls for its FreeBSD personality.
        __KER_OSCRAFT_THR_NEW => crate::personality::native::thr_new(a(0), a(1), a(2) as u32, a(3) as i64),
        __KER_OSCRAFT_THR_SELF => crate::personality::native::thr_self(),
        __KER_OSCRAFT_FUTEX => crate::personality::native::futex(a(0), a(1) as u32, a(2) as u32, a(3) as u32),
        __KER_PIPE_OPEN => kpipe::pipe_open(a(0) as i32, a(1) as i32, a(2), a(3) as u32),
        __KER_PIPE_CTL => kpipe::pipe_ctl(a(0) as i32, a(1)),
        __KER_PIPE_CLOSE => kpipe::pipe_close(a(0) as i32),
        __KER_MSG_QUEUE_OPEN => mqueue::msg_queue_open(
            a(0) as i64,
            a(1) as i32,
            a(2) as u32,
            a(3) as u32,
            a(4) as u32,
        ),
        __KER_MSG_QUEUE_SEND => mqueue::msg_queue_send(a(0) as i32, a(1), a(2), a(3) as u32),
        __KER_MSG_QUEUE_RECEIVE => mqueue::msg_queue_receive(a(0) as i32, a(1), a(2), a(3)),
        __KER_MSG_QUEUE_CTL => mqueue::msg_queue_ctl(a(0) as i32, a(1) as u32, a(2)),
        __KER_MSG_QUEUE_CLOSE => mqueue::msg_queue_close(a(0) as i64, a(1) as i32),
        __KER_THREAD_CANCEL => thread::thread_cancel(a(0) as u32, a(1)),
        __KER_SYNC_MUTEX_LOCK => sync::mutex_lock(a(0)),
        __KER_SYNC_MUTEX_UNLOCK => sync::mutex_unlock(a(0)),
        __KER_SYNC_CONDVAR_WAIT => sync::condvar_wait(a(0), a(1)),
        __KER_SYNC_CONDVAR_SIGNAL => sync::condvar_signal(a(0), a(1) != 0),
        __KER_SYNC_CREATE => sync::sync_create(a(0) as u32, a(1), a(2)),
        __KER_SCHED_YIELD => sched_yield(),
        __KER_SCHED_CTL => sched_ctl(a(0) as u32, a(1), a(2)),
        __KER_TIMER_DELEGATE => offline::timer_delegate(a(0) as u32, a(1) as i32, a(2) as i32),
        __KER_SCHED_INFO => sched_info(a(0) as i32, a(1) as u32, a(2)),
        __KER_SCHED_GET => sched_get(a(0) as i32, a(1) as u32, a(2)),
        __KER_SCHED_SET => sched_set(a(0) as i32, a(1) as u32, a(2) as u32, a(3)),
        __KER_INTERRUPT_ATTACH => {
            interrupt::interrupt_attach(a(0) as u32, a(1), a(2), a(3), a(4) as u32)
        }
        __KER_INTERRUPT_DETACH => interrupt::interrupt_detach(a(0) as i32),
        __KER_INTERRUPT_MASK => interrupt::interrupt_mask(a(0) as u32, a(1) as i32),
        __KER_INTERRUPT_UNMASK => interrupt::interrupt_unmask(a(0) as u32, a(1) as i32),
        __KER_INTERRUPT_WAIT => interrupt::interrupt_wait(a(0) as u32, a(1)),
        __KER_INTERRUPT_UNBLOCK => interrupt::interrupt_unblock(a(0) as i32),
        __KER_INTERRUPT_QUERY => interrupt::interrupt_query(a(0) as u32, a(1) as i32, a(2), a(3)),
        // Undocumented; procnto's ker_interrupt_ctl() has no commands.
        __KER_INTERRUPT_CTL => Err(errno::ENOSYS),
        // x0 bit 0: unmask, as `_NTO_INTR_WAIT_FLAGS_UNMASK`.
        __MUON_CALL_INTERRUPT_WAIT => interrupt::interrupt_wait(
            if a(0) & 1 != 0 { interrupt::WAIT_FLAGS_UNMASK } else { 0 },
            0,
        ),
        n => {
            let me = sched::current();
            if me.process.pid != 1 {
                crate::kprintln!(
                    "[pid {}] kernel call {n} not implemented (x0..x3 {:#x} {:#x} {:#x} {:#x}, from {:#x})",
                    me.process.pid,
                    a(0),
                    a(1),
                    a(2),
                    a(3),
                    frame.elr
                );
            }
            Err(errno::ENOSYS)
        }
    };
    // A TimerTimeout applies to the next kernel call only.
    if num != number::__KER_TIMER_TIMEOUT {
        clear_timeout();
    }
    sched::current().kcall_preemptible.store(outer, Ordering::Relaxed);
    trace_exit(num, frame, &result);
    trace::kick();
    if point && result == Err(errno::EINTR) && thread::cancellation_point(frame) {
        user_return(frame);
        return;
    }
    // `SA_RESTART`: back to the `svc`, its arguments still in the frame,
    // and the handler returns to it. The calls FreeBSD restarts are
    // messages to servers and lock waits.
    if result == Err(errno::EINTR)
        && matches!(num, number::__KER_MSG_SENDV | number::__KER_MSG_SENDVNC | number::__KER_OSCRAFT_FUTEX)
        && sched::current().process.personality() == process::Personality::Native
        && signal::restarts()
    {
        frame.elr -= 4;
        user_return(frame);
        return;
    }
    let result = result.map_err(|e| sched::current().process.personality().errno_out(e));
    frame.complete_kercall(result);
    user_return(frame);
}

/// The calling thread's armed `TimerTimeout()`, if any, is used up.
fn clear_timeout() {
    let me = sched::current();
    if me.timeout_armed.swap(false, Ordering::Relaxed) {
        *me.armed_timeout.lock() = None;
    }
}

/// Is kernel call `num` a cancellation point? Those that block, except
/// `MsgSendvnc()` and `SyncMutexLock()` (`ThreadCancel()`'s page); a
/// `TimerTimeout()` only for a sleep, a semaphore wait unless it only tries.
fn is_cancellation_point(num: u32, frame: &TrapFrame) -> bool {
    use number::*;
    match num {
        __KER_MSG_SENDV
        | __KER_MSG_RECEIVEV
        | __KER_MSG_RECEIVEPULSEV
        | __KER_SIGNAL_SUSPEND
        | __KER_SIGNAL_WAITINFO
        | __KER_SYNC_CONDVAR_WAIT
        | __KER_SYNC_BARRIER_WAIT
        | __KER_MSG_QUEUE_SEND
        | __KER_MSG_QUEUE_RECEIVE
        | __KER_THREAD_JOIN
        | __KER_INTERRUPT_WAIT
        | __MUON_CALL_INTERRUPT_WAIT => true,
        __KER_SYNC_SEM_WAIT => frame.kercall_arg(1) == 0,
        __KER_TIMER_TIMEOUT => frame.kercall_arg(1) as u32 & timer::TIMEOUT_NANOSLEEP != 0,
        _ => false,
    }
}

/// Trace a kernel call's entry (`_NTO_TRACE_KERCALLENTER`): in fast mode
/// the two words QNX's tables give for the call (mostly its first two
/// arguments), in wide mode its first six arguments.
fn trace_enter(num: u32, frame: &TrapFrame) {
    use number::*;
    let ev = num & 0x7f;
    if !trace::on(trace::C_KERCALL, ev) {
        return;
    }
    let a = |i| frame.kercall_arg(i);
    if trace::wide(trace::C_KERCALL, ev) {
        let d: [u32; 6] = core::array::from_fn(|i| a(i) as u32);
        trace::emit(trace::C_KERCALL, ev, &d);
        return;
    }
    let d = match num {
        // coid, the message's first word
        __KER_MSG_SENDV | __KER_MSG_SENDVNC => [a(0) as u32, first_word(a(1), a(2) as i64)],
        // pid, signo
        __KER_SIGNAL_KILL => [a(1) as u32, a(3) as u32],
        // signo, the handler
        __KER_SIGNAL_ACTION => [a(2) as u32, user_u64(a(3)) as u32],
        // flags, the time (seconds)
        __KER_TIMER_TIMEOUT => [a(1) as u32, (user_u64(a(3)) / 1_000_000_000) as u32],
        // id, the time (seconds)
        __KER_TIMER_SETTIME => [a(0) as u32, (user_u64(a(2)) / 1_000_000_000) as u32],
        _ => [a(0) as u32, a(1) as u32],
    };
    trace::emit(trace::C_KERCALL, ev, &d);
}

/// Trace a kernel call's exit (`_NTO_TRACE_KERCALLEXIT`): its result (with
/// the first word of the message received or replied for the message
/// calls), or -1 and the errno.
fn trace_exit(num: u32, frame: &TrapFrame, result: &KerResult) {
    use number::*;
    let ev = (num & 0x7f) + trace::KERCALL_EXIT;
    if !trace::on(trace::C_KERCALL, ev) {
        return;
    }
    let a = |i| frame.kercall_arg(i);
    let d = match (num, result) {
        (_, Err(e)) => [u32::MAX, *e],
        (__KER_MSG_RECEIVEV | __KER_MSG_RECEIVEPULSEV, Ok(v)) => {
            [*v as u32, first_word(a(1), a(2) as i64)]
        }
        (__KER_MSG_SENDV | __KER_MSG_SENDVNC, Ok(v)) => [*v as u32, first_word(a(3), a(4) as i64)],
        (_, Ok(v)) => [*v as u32, 0],
    };
    trace::emit(trace::C_KERCALL, ev, &d);
}

/// The first word of the message in buffer `addr` (`parts` < 0: one
/// buffer of -`parts` bytes, else an IOV array of `parts`), 0 if none.
fn first_word(addr: u64, parts: i64) -> u32 {
    let space = umem::Space::current();
    let buf = if parts < 0 {
        (-parts >= 4).then_some(addr)
    } else if parts > 0 {
        let base = user_u64(addr);
        let len = user_u64(addr + 8);
        (len >= 4).then_some(base)
    } else {
        None
    };
    let mut b = [0u8; 4];
    match buf {
        Some(at) if umem::read(space, at, &mut b).is_ok() => u32::from_le_bytes(b),
        _ => 0,
    }
}

/// A u64 of the caller's memory (0 if unreadable or null).
fn user_u64(addr: u64) -> u64 {
    let mut b = [0u8; 8];
    if addr == 0 || umem::read(umem::Space::current(), addr, &mut b).is_err() {
        return 0;
    }
    u64::from_le_bytes(b)
}

/// The calling thread is about to return to EL0 (after a kernel call or
/// an interrupt taken there): it waits while its process is stopped (see
/// `debug`), ends if it has to (`exit_on_return`: its process is ending,
/// or exec() replaced it), else takes its pending signals.
pub fn user_return(frame: &mut TrapFrame) {
    let me = sched::current();
    if me.process.pid != process::KERNEL_PID {
        // Stopped by job control or a debugger: wait here.
        debug::park_if_stopped(me);
    }
    if me.exit_on_return.load(Ordering::Relaxed) {
        sched::exit();
    }
    signal::deliver(frame);
    debug::before_eret(frame);
}

/// Kernel work a user thread does before it first runs at EL0 (called
/// from `thread_start_user` with its initial user state).
#[unsafe(no_mangle)]
extern "C" fn thread_user_entry(frame: &mut TrapFrame) {
    sched::started();
    fork_startup(frame);
    debug::before_eret(frame);
    crate::arch::exception::set_align_check(sched::current().align_fault.load(Ordering::Relaxed));
}

/// A file a spawned child opens itself as it starts, as procnto's loader
/// thread does for an open file action naming another server's file (see
/// `fork_startup`).
pub struct StartupOpen {
    /// The descriptor, and those that are to be copies of it (dup2()
    /// actions after the open).
    pub fd: i32,
    pub copies: alloc::vec::Vec<i32>,
    /// The server (pid, chid), its mount point's handle, and the path
    /// below it.
    pub pid: i32,
    pub chid: i32,
    pub handle: u32,
    pub path: alloc::string::String,
    pub ioflag: u32,
    pub mode: u32,
}

/// A forked or spawned child's start-up, as procnto's loader thread does
/// it before the parent's `fork()` or `spawn()` returns: tell the servers
/// (other than procnto) that the child has copies of the parent's
/// connections, with an `_IO_DUP` on each naming the parent's connection
/// in its `struct _msg_info_hdr` (the child's client info shows
/// `_NTO_CI_LOADER` meanwhile, which servers require for that); open the
/// files of `pending_opens`; then let process 1 answer the parent (the
/// pulse's value: the pid, and from bit 32 an errno that fails the spawn).
/// A failed open ends the child, which leaves no zombie. The messages are
/// built on the user stack, below the stack pointer.
fn fork_startup(frame: &TrapFrame) {
    let me = sched::current();
    let dups = me.pending_dups.lock().take();
    let opens = me.pending_opens.lock().take();
    let closes = me.pending_closes.lock().take();
    if dups.is_none() && opens.is_none() && closes.is_none() {
        return;
    }
    let at = (frame.sp_el0 - 512) & !15;
    // An exec's close-on-exec descriptors of other servers: `_IO_CLOSE`
    // to each, as libc's close() would have sent, then the connection
    // goes (review PROCMGR-8). Nothing else to do for an exec.
    if let Some(closes) = closes {
        for coid in closes {
            let _ = io_close(at, coid);
            drop(me.process.remove_connection(coid));
        }
        if dups.is_none() && opens.is_none() {
            return;
        }
    }
    me.process.loading.store(true, Ordering::Relaxed);
    for [coid, ppid, chid, scoid, pcoid] in dups.unwrap_or_default() {
        let _ = io_dup(at, coid, ppid, chid, scoid, pcoid);
    }
    me.process.loading.store(false, Ordering::Relaxed);
    let err = match opens.map_or(Ok(()), |o| startup_opens(at, o)) {
        Ok(()) => 0,
        Err(e) => {
            // As if it never was: the spawn fails, nobody waits for it.
            me.process.nozombie.store(true, Ordering::Relaxed);
            me.process.ppid.store(process::KERNEL_PID, Ordering::Relaxed);
            e
        }
    };
    let _ = msg::msg_send_pulse(
        process::SIDE_CHANNEL as i32,
        -1,
        process::PULSE_FORK_DONE,
        me.process.pid as u32 as u64 | (err as u64) << 32,
    );
    if err != 0 {
        process::exit_current(signal::CLD_EXITED, 127);
    }
}

/// Send an `_IO_DUP` on `coid` (built at `at`) saying that it copies
/// connection `from_coid` of process `pid` (to channel `chid`, server
/// connection `scoid`).
/// `_IO_CLOSE` on `coid`: `struct _io_close` { type, combine_len }.
fn io_close(at: u64, coid: i32) -> KerResult {
    const IO_CLOSE: u16 = 0x116;
    let mut m = [0u8; 4];
    m[0..2].copy_from_slice(&IO_CLOSE.to_le_bytes());
    m[2..4].copy_from_slice(&4u16.to_le_bytes());
    umem::write(umem::Space::current(), at, &m)?;
    msg::msg_sendv(coid, at, -4, 0, 0)
}

fn io_dup(at: u64, coid: i32, pid: i32, chid: i32, scoid: i32, from_coid: i32) -> KerResult {
    const IO_DUP: u16 = 0x115;
    let mut m = [0u8; 40];
    m[0..2].copy_from_slice(&IO_DUP.to_le_bytes());
    m[2..4].copy_from_slice(&40u16.to_le_bytes());
    m[12..16].copy_from_slice(&pid.to_le_bytes()); // info.pid
    m[20..24].copy_from_slice(&chid.to_le_bytes()); // info.chid
    m[24..28].copy_from_slice(&scoid.to_le_bytes()); // info.scoid
    m[28..32].copy_from_slice(&from_coid.to_le_bytes()); // info.coid
    umem::write(umem::Space::current(), at, &m)?;
    msg::msg_sendv(coid, at, -40, 0, 0)
}

/// Open the files of `opens` as libc's `open()` and `dup2()` would: a
/// connection at the descriptor and an `_IO_CONNECT` (built at `at`, down
/// from there), then for each copy, a connection and an `_IO_DUP`. The
/// first error.
fn startup_opens(at: u64, opens: alloc::vec::Vec<StartupOpen>) -> Result<(), u32> {
    let me = sched::current();
    for o in opens {
        // The flags are QNX's, as the spawn request's: right for a QNX
        // child, which `qnxd` serves, and for a QNX server; a FreeBSD
        // child talks to a native server directly.
        let native = |p: &process::Process| p.personality() == process::Personality::Native;
        let ioflag = if native(&me.process) && process::lookup(o.pid).is_some_and(|p| native(&p)) {
            crate::personality::qnx::ioflag_in(o.ioflag)
        } else {
            o.ioflag
        };
        let m = crate::proc::client::connect_msg(o.handle, ioflag, o.mode, &o.path);
        let at = (at - m.len() as u64) & !15;
        let coid = channel::connect_attach(0, o.pid, o.chid, o.fd as u32, 0)? as i32;
        umem::write(umem::Space::current(), at, &m)?;
        msg::msg_sendv(coid, at, -(m.len() as i64), 0, 0)?;
        let scoid = me.process.connection(coid).ok_or(errno::EBADF)?.scoid;
        for to in o.copies {
            let copy = channel::connect_attach(0, o.pid, o.chid, to as u32, 0)? as i32;
            io_dup(at, copy, me.process.pid, o.chid, scoid, coid)?;
        }
    }
    Ok(())
}

// ANCHOR: unblock
/// Make thread `t`, blocked in a kernel call, leave it with `err` (a
/// signal: EINTR; a timeout: ETIMEDOUT), whatever it waits in. Returns
/// whether it was released (a server with `_NTO_CHF_UNBLOCK` may first
/// be asked).
pub fn unblock(t: &sched::Thread, err: u32) -> bool {
    use sched::State;
    match t.state() {
        State::Send | State::Receive | State::Reply => msg::unblock(t, err),
        State::Mutex | State::Condvar | State::Sem | State::Barrier => sync::unblock(t, err),
        State::MqSend | State::MqReceive => mqueue::unblock(t, err),
        State::Pipe => kpipe::unblock(t, err),
        State::Nanosleep
        | State::Sigsuspend
        | State::Sigwaitinfo
        | State::Intr
        | State::Join
        | State::Stopped
        | State::TraceBuffer => sched::wake_with(t, err),
        _ => false,
    }
}
// ANCHOR_END: unblock

/// Kernel calls handled so far.
pub fn calls() -> u64 {
    (0..crate::smp::online_count())
        .filter_map(crate::percpu::get)
        .map(|c| c.kercalls.load(Ordering::Relaxed))
        .sum::<u64>()
}

/// `__SysCpupageGet(index)`: CPUPAGE_PLS (1), CPUPAGE_SYSPAGE (2, the
/// system page's user address), CPUPAGE_CPU (3); EINVAL for the others.
fn sys_cpupage_get(index: i32) -> KerResult {
    match index {
        1 => Ok(process::current().pls.load(Ordering::Relaxed)),
        2 => Ok(crate::proc::loader::user_syspage_ptr()),
        3 => Ok(crate::percpu::this().cpu as u64),
        _ => Err(errno::EINVAL),
    }
}

/// `__SysCpupageSet(index, value)`: only CPUPAGE_PLS (1).
fn sys_cpupage_set(index: i32, value: u64) -> KerResult {
    if index != 1 {
        return Err(errno::EINVAL);
    }
    process::current().pls.store(value, Ordering::Relaxed);
    Ok(0)
}

/// `ThreadDestroy(tid, priority, status)`: tid -1 ends the whole process
/// with exit status `status` (libc's `_exit()`); 0 or the caller's tid
/// ends the calling thread; another thread of the process ends with
/// `status` as it next returns to user mode, woken if it is blocked (as
/// procnto: ESRCH if there is none, EINVAL below -1).
fn thread_destroy(tid: i32, status: u64) -> KerResult {
    let me = sched::current();
    if tid < -1 {
        return Err(errno::EINVAL);
    }
    if tid > 0 && tid as u32 != me.ptid {
        let t = sched::threads_of(&me.process)
            .into_iter()
            .find(|t| t.ptid == tid as u32)
            .ok_or(errno::ESRCH)?;
        t.exit_status.store(status, Ordering::Relaxed);
        t.exit_on_return.store(true, Ordering::Relaxed);
        if t.state().is_blocked() {
            unblock(&t, errno::EINTR);
        }
        // Busy in user code on another CPU: it ends now, not at that
        // CPU's next tick (review SCHED-10; procnto sends an IPI).
        sched::interrupt(&t);
        return Ok(0);
    }
    me.exit_status.store(status, Ordering::Relaxed);
    if tid == -1 && me.process.pid != process::KERNEL_PID {
        process::exit_current(signal::CLD_EXITED, status as i32);
    }
    if tid == 0 || tid == -1 || tid as u32 == me.ptid {
        // A stack the kernel mapped for it goes with it.
        if let Some((addr, len)) = me.owned_stack.lock().take() {
            let _ = crate::vm::map::unmap(&me.process, addr, len);
        }
        sched::exit();
    }
    Err(errno::ENOSYS)
}

// Scheduling policies (`<sched.h>`).
const SCHED_NOCHANGE: u32 = 0;
const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
const SCHED_OTHER: u32 = 3;
const SCHED_SPORADIC: u32 = 4;
const SCHED_SETPRIO: u32 = 7;
const SCHED_HIGH_PRIORITY_IST: u32 = 8;
const SCHED_OFFLINING: u32 = 9;
/// `sizeof(struct sched_param)`: `sched_priority`, `sched_curpriority`,
/// then the sporadic-server fields.
const SCHED_PARAM_SIZE: usize = 48;

/// The thread `SchedGet()`/`SchedSet()` name: thread `tid` of process
/// `pid` (0: the caller's), or the calling thread if `tid` is 0.
fn sched_target(pid: i32, tid: u32) -> Result<sched::ThreadRef, u32> {
    if tid == 0 {
        return Ok(sched::current().arc());
    }
    let p = if pid == 0 {
        process::current().arc()
    } else {
        process::lookup(pid).ok_or(errno::ESRCH)?
    };
    sched::thread_of(&p, tid).ok_or(errno::ESRCH)
}

/// `SchedGet(pid, tid, param)`: the thread's policy; its priorities in
/// `param` (if set).
fn sched_get(pid: i32, tid: u32, param: u64) -> KerResult {
    let t = sched_target(pid, tid)?;
    ability::may_schedule(process::current(), &t.process)?;
    if param != 0 {
        let mut p = [0u8; SCHED_PARAM_SIZE];
        p[0..4].copy_from_slice(&(t.base_priority() as i32).to_le_bytes());
        p[4..8].copy_from_slice(&(t.priority() as i32).to_le_bytes());
        // A sporadic thread's parameters; its normal priority while it
        // runs at its low one.
        if t.policy() == sched::Policy::Sporadic
            && let Some((ss, normal)) = crate::sporadic::get(&t)
        {
            if let Some(prio) = normal {
                p[0..4].copy_from_slice(&(prio as i32).to_le_bytes());
            }
            p[8..12].copy_from_slice(&(ss.low_priority as i32).to_le_bytes());
            p[12..16].copy_from_slice(&(ss.max_repl as i32).to_le_bytes());
            let ts = |ns: u64| ((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as i64);
            let (s, n) = ts(ss.repl_period);
            p[16..24].copy_from_slice(&s.to_le_bytes());
            p[24..32].copy_from_slice(&n.to_le_bytes());
            let (s, n) = ts(ss.init_budget);
            p[32..40].copy_from_slice(&s.to_le_bytes());
            p[40..48].copy_from_slice(&n.to_le_bytes());
        }
        umem::write(umem::Space::current(), param, &p)?;
    }
    Ok(policy_number(t.policy()) as u64)
}

/// The `SCHED_*` number of a policy.
pub fn policy_number(p: sched::Policy) -> u32 {
    match p {
        sched::Policy::Fifo => SCHED_FIFO,
        sched::Policy::RoundRobin => SCHED_RR,
        sched::Policy::Other => SCHED_OTHER,
        sched::Policy::Sporadic => SCHED_SPORADIC,
        sched::Policy::HighPriorityIst => SCHED_HIGH_PRIORITY_IST,
        sched::Policy::Offlining => SCHED_OFFLINING,
    }
}

/// The highest priority of `SCHED_*` policy `policy` (`SchedSet()`,
/// `SchedInfo()`): `SCHED_OFFLINING` and `SCHED_HIGH_PRIORITY_IST`
/// threads may go up to 254, the others stay below the clock.
fn policy_max_priority(policy: u32) -> u8 {
    match policy {
        SCHED_HIGH_PRIORITY_IST | SCHED_OFFLINING => ability::PRIORITY_MAX_HIGH,
        _ => ability::priority_max(),
    }
}

/// The scheduling a new thread is created with, when its creator names it
/// (`personality::native::thr_new`, FreeBSD's `thr_param.rtp`): the policy
/// and priority `SchedSet()` would accept from the caller for one of its
/// own threads, so that the thread never runs at any other.
pub(crate) fn new_thread_sched(policy: u32, priority: i64) -> Result<(sched::Policy, u8), u32> {
    let p = match policy {
        SCHED_FIFO => sched::Policy::Fifo,
        SCHED_RR => sched::Policy::RoundRobin,
        SCHED_OTHER => sched::Policy::Other,
        _ => return Err(errno::EINVAL),
    };
    let prio = ability::priority_up_to(process::current(), priority, policy_max_priority(policy))?;
    Ok((p, prio))
}

/// `SchedSet(pid, tid, policy, param)`: set the thread's policy (unless
/// `SCHED_NOCHANGE` or `SCHED_SETPRIO`) and priority. `SCHED_OTHER` is
/// scheduled as round robin; `SCHED_SPORADIC` takes the sporadic-server
/// parameters (see `sporadic`). As procnto: tid -1 is ENOSYS;
/// `SCHED_HIGH_PRIORITY_IST` is only for the calling thread and only if
/// it is an interrupt thread (`InterruptAttachThread()`), and
/// `SCHED_OFFLINING` only for the calling thread, with
/// `PROCMGR_AID_RUNSTATE` (EINVAL, EPERM); both go up to 254.
fn sched_set(pid: i32, tid: u32, policy: u32, param: u64) -> KerResult {
    let mut p = [0u8; SCHED_PARAM_SIZE];
    umem::read(umem::Space::current(), param, &mut p)?;
    if tid == u32::MAX {
        return Err(errno::ENOSYS);
    }
    // process 1's own threads are never changed from outside, not even by
    // root: EPERM before the thread is looked up (procnto's
    // `sched_thread_lookup`; measured, also for a thread it doesn't have).
    if pid == process::KERNEL_PID && tid != 0 && process::current().pid != process::KERNEL_PID {
        return Err(errno::EPERM);
    }
    let t = sched_target(pid, tid)?;
    ability::may_schedule(process::current(), &t.process)?;
    let i32_at = |o: usize| i32::from_le_bytes(p[o..o + 4].try_into().unwrap());
    let i64_at = |o: usize| i64::from_le_bytes(p[o..o + 8].try_into().unwrap());
    let me = sched::current();
    let number = match policy {
        SCHED_NOCHANGE | SCHED_SETPRIO => policy_number(t.policy()),
        SCHED_FIFO | SCHED_RR | SCHED_OTHER | SCHED_SPORADIC => policy,
        SCHED_HIGH_PRIORITY_IST if core::ptr::eq(&*t, me) && interrupt::is_ist(me) => policy,
        SCHED_OFFLINING if core::ptr::eq(&*t, me) => {
            ability::check(ability::RUNSTATE, None)?;
            policy
        }
        _ => return Err(errno::EINVAL),
    };
    let prio = ability::priority_up_to(process::current(), i32_at(0) as i64, policy_max_priority(number))?;
    let policy = match policy {
        // Only the priority (a sporadic thread at its low priority keeps
        // it as its normal one too).
        SCHED_SETPRIO => {
            if t.policy() == sched::Policy::Sporadic {
                crate::sporadic::set_prio(&t, prio);
            }
            sched::set_base_priority(&t, prio);
            return Ok(0);
        }
        SCHED_NOCHANGE => t.policy(),
        SCHED_FIFO => sched::Policy::Fifo,
        SCHED_RR => sched::Policy::RoundRobin,
        SCHED_OTHER => sched::Policy::Other,
        SCHED_SPORADIC => sched::Policy::Sporadic,
        SCHED_HIGH_PRIORITY_IST => sched::Policy::HighPriorityIst,
        _ => sched::Policy::Offlining,
    };
    let run_at = if policy == sched::Policy::Sporadic {
        let params = crate::sporadic::Params {
            low_priority: i32_at(8),
            max_repl: i32_at(12),
            repl_period: (i64_at(16), i64_at(24)),
            init_budget: (i64_at(32), i64_at(40)),
        };
        crate::sporadic::validate(prio, &params)?;
        crate::sporadic::set(&t, prio, &params)
    } else {
        crate::sporadic::clear(&t);
        prio
    };
    t.set_policy(policy);
    sched::set_base_priority(&t, run_at);
    sched::reassess(&t);
    Ok(0)
}

/// `SchedInfo(pid, policy, info)`: `struct _sched_info` { priority_min,
/// priority_max, interval (ns), priority_priv, reserved[11] } of a
/// scheduling policy: priorities from 1 to the policy's highest (below
/// the clock's, 254 for `SCHED_OFFLINING` and `SCHED_HIGH_PRIORITY_IST`)
/// for a process with `PROCMGR_AID_PRIORITY`, else to 63 (as procnto), the round-robin
/// timeslice (`SCHED_RR`, `SCHED_OTHER`; 0 for the others), and the
/// highest priority without the ability (procnto's `priv_prio` less one).
fn sched_info(pid: i32, policy: u32, info: u64) -> KerResult {
    if pid != 0 && process::lookup(pid).is_none() {
        return Err(errno::ESRCH);
    }
    let interval = match policy {
        SCHED_FIFO | SCHED_SPORADIC | SCHED_HIGH_PRIORITY_IST | SCHED_OFFLINING => 0,
        SCHED_RR | SCHED_OTHER => sched::rr_slice_ns(),
        _ => return Err(errno::EINVAL),
    };
    let mut b = [0u8; 64];
    b[0..4].copy_from_slice(&1i32.to_le_bytes());
    let max = if ability::allowed(process::current(), ability::PRIORITY, None) {
        policy_max_priority(policy)
    } else {
        ability::PRIORITY_MAX_NONPRIV
    };
    b[4..8].copy_from_slice(&(max as i32).to_le_bytes());
    b[8..16].copy_from_slice(&interval.to_le_bytes());
    b[16..20].copy_from_slice(&(ability::PRIORITY_MAX_NONPRIV as i32).to_le_bytes());
    umem::write(umem::Space::current(), info, &b)?;
    Ok(0)
}

/// `SchedCtl()` commands (`<sys/neutrino.h>`).
const SCHED_PROCESSOR_ONLINE: u32 = 4;
const SCHED_PROCESSOR_OFFLINE: u32 = 5;
const SCHED_PROCESSORS_STATUS: u32 = 6;

/// `SchedCtl(cmd, data, length)`. `SCHED_PROCESSORS_STATUS` fills a
/// `struct _sched_processors_status` (16 bytes): the number of CPUs, the
/// number online, and the mask of those online (not taken offline).
/// `SCHED_PROCESSOR_OFFLINE`/`ONLINE`: see `offline`.
fn sched_ctl(cmd: u32, data: u64, length: u64) -> KerResult {
    match cmd {
        SCHED_PROCESSORS_STATUS => {
            if length != 16 {
                return Err(errno::EINVAL);
            }
            let present = crate::syspage::get().header().num_cpu as u32;
            let mask = offline::online_mask();
            let mut b = [0u8; 16];
            b[0..4].copy_from_slice(&present.to_le_bytes());
            b[4..8].copy_from_slice(&mask.count_ones().to_le_bytes());
            b[8..16].copy_from_slice(&mask.to_le_bytes());
            umem::write(umem::Space::current(), data, &b)?;
            Ok(0)
        }
        SCHED_PROCESSOR_ONLINE | SCHED_PROCESSOR_OFFLINE => offline::set_online(cmd == SCHED_PROCESSOR_ONLINE),
        _ => Err(errno::ENOSYS),
    }
}

/// `SchedYield()`: let other ready threads of the same priority run.
fn sched_yield() -> KerResult {
    sched::yield_now();
    Ok(0)
}

/// `CacheControl(addr, len, flags)`: `msync()`'s cache operations on the
/// caller's memory (MS_SYNC/MS_ASYNC, MS_INVALIDATE,
/// MS_INVALIDATE_ICACHE), done by the kernel without a message. Pages
/// that aren't mapped are skipped, not an error (as procnto, despite the
/// documented EFAULT).
fn cache_control(addr: u64, len: u64, flags: u32) -> KerResult {
    let me = process::current();
    let end = addr.saturating_add(len);
    let page = crate::vm::pmem::PAGE_SIZE;
    let mut va = addr;
    while va < end {
        let next = ((va & !(page - 1)) + page).min(end);
        let _ = crate::proc::memmgr::sync(&me, va, next - va, flags);
        va = next;
    }
    Ok(0)
}

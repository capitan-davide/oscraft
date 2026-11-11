//! The kernel calls: the number in `w8`, then `svc #0x51`. The kernel
//! returns to the instruction after the `svc` on success and to the one
//! after that on failure, with the error number (QNX's) in `x0`.

use core::arch::asm;
use core::ffi::c_void;

/// `__KER_MSG_SENDV` and `__KER_THREAD_DESTROY` (`<sys/kercalls.h>`).
const MSG_SENDV: u32 = 11;
const THREAD_DESTROY: u32 = 47;

/// A part of a message, `iov_t`: the layout `MsgSendv()` takes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Iov {
    pub base: *const c_void,
    pub len: usize,
}

impl Iov {
    pub fn of<T: proto::pod::Pod>(v: &T) -> Self {
        Iov {
            base: (v as *const T).cast(),
            len: size_of::<T>(),
        }
    }

    pub fn raw(base: *const c_void, len: usize) -> Self {
        Iov { base, len }
    }
}

/// A kernel call with up to five arguments: the result, or QNX's error.
///
/// # Safety
///
/// The arguments must be what the call expects: pointers valid for it.
unsafe fn kercall(nr: u32, a: [u64; 5]) -> Result<u64, i32> {
    let (r, failed): (u64, u64);
    // SAFETY: the trap; the caller vouches for the arguments.
    unsafe {
        asm!(
            "mov x9, #0",
            "svc #0x51",
            "b 1f",
            "mov x9, #1",
            "1:",
            in("w8") nr,
            inlateout("x0") a[0] => r,
            in("x1") a[1], in("x2") a[2], in("x3") a[3], in("x4") a[4],
            out("x9") failed,
            clobber_abi("C"),
        );
    }
    if failed == 0 { Ok(r) } else { Err(r as i32) }
}

/// `MsgSendv(coid, send, reply)`: the server's reply status.
///
/// # Safety
///
/// Every part is valid for its length while the call lasts.
pub unsafe fn msg_sendv(coid: i32, send: &[Iov], reply: &[Iov]) -> Result<i64, i32> {
    // SAFETY: the caller's guarantee.
    unsafe {
        kercall(
            MSG_SENDV,
            [
                coid as u64,
                send.as_ptr() as u64,
                send.len() as u64,
                reply.as_ptr() as u64,
                reply.len() as u64,
            ],
        )
    }
    .map(|r| r as i64)
}

/// `ThreadDestroy(-1, -1, status)`: every thread, the process with them.
pub fn thread_destroy_all(status: i32) {
    thread_destroy(-1, -1, status);
}

/// `ThreadDestroy(tid, prio, status)`.
pub fn thread_destroy(tid: i32, prio: i32, status: i32) {
    // SAFETY: no pointers.
    let _ = unsafe { kercall(THREAD_DESTROY, [tid as i64 as u64, prio as i64 as u64, status as i64 as u64, 0, 0]) };
}

/// Sends `msg` and takes a reply of type `R` (`MsgSendv` with one part
/// each way): the status and the reply.
pub fn ask<T: proto::pod::Pod, R: proto::pod::Pod>(
    coid: i32,
    msg: &T,
) -> Result<(i64, R), i32> {
    let mut reply = R::zeroed();
    let send = [Iov::of(msg)];
    let recv = [Iov::raw((&raw mut reply).cast(), size_of::<R>())];
    // SAFETY: both are on this stack, valid for their sizes; any bytes
    // are an R (Pod).
    let status = unsafe { msg_sendv(coid, &send, &recv) }?;
    Ok((status, reply))
}

/// Sends `msg` with no reply data: the status.
pub fn send<T: proto::pod::Pod>(coid: i32, msg: &T) -> Result<i64, i32> {
    let send = [Iov::of(msg)];
    // SAFETY: the message is on this stack.
    unsafe { msg_sendv(coid, &send, &[]) }
}

/// The connection to the process manager every process has
/// (`SYSMGR_COID`, `<sys/neutrino.h>`).
pub const SYSMGR_COID: i32 = 0x4000_0000;

/// `__KER_CONNECT_ATTACH` and `__KER_CONNECT_DETACH`.
const CONNECT_ATTACH: u32 = 39;
const CONNECT_DETACH: u32 = 40;
/// `__KER_CLOCK_TIME`.
const CLOCK_TIME: u32 = 65;

/// `ConnectAttach(nd, pid, chid, index, flags)`: a connection, from
/// `index` up (0 for a file descriptor), to the channel.
pub fn connect_attach(nd: u32, pid: i32, chid: i32, index: u32, flags: i32) -> Result<i32, i32> {
    // SAFETY: no pointers.
    unsafe {
        kercall(
            CONNECT_ATTACH,
            [u64::from(nd), pid as u64, chid as u64, u64::from(index), flags as u64],
        )
    }
    .map(|c| c as i32)
}

/// `ConnectDetach(coid)`.
pub fn connect_detach(coid: i32) -> Result<(), i32> {
    // SAFETY: no pointers.
    unsafe { kercall(CONNECT_DETACH, [coid as u64, 0, 0, 0, 0]) }.map(|_| ())
}

/// `ClockTime(id, NULL, &old)`: clock `id`'s time in nanoseconds.
pub fn clock_time(id: i32) -> Result<u64, i32> {
    let mut old: u64 = 0;
    // SAFETY: `old` is on this stack for the call.
    unsafe { kercall(CLOCK_TIME, [id as u64, 0, (&raw mut old) as u64, 0, 0]) }?;
    Ok(old)
}

/// `__KER_SIGNAL_PROCMASK` and `__KER_SIGNAL_KILL`.
const SIGNAL_PROCMASK: u32 = 30;
const SIGNAL_KILL: u32 = 26;
const MSG_SEND_PULSE: u32 = 20;

/// `SignalProcmask(pid, tid, how, set, oldset)` on the calling thread.
///
/// # Safety
///
/// `set` is a QNX `sigset_t` or null; `oldset` likewise.
pub unsafe fn signal_procmask(how: i32, set: *const [u32; 4], oldset: *mut [u32; 4]) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(SIGNAL_PROCMASK, [0, 0, how as u64, set as u64, oldset as u64]) }.map(|_| ())
}

/// `MsgSendPulse(coid, priority, code, value)`.
pub fn msg_send_pulse(coid: i32, priority: i32, code: i32, value: i64) -> Result<(), i32> {
    // SAFETY: no pointers.
    unsafe { kercall(MSG_SEND_PULSE, [coid as u64, priority as u64, code as u64, value as u64, 0]) }.map(|_| ())
}

/// `SignalKill(0, pid, tid, signo, code, value)`.
pub fn signal_kill(pid: i32, tid: i32, signo: i32, code: i32, value: i32) -> Result<(), i32> {
    // SAFETY: no pointers; five arguments after the node.
    unsafe {
        kercall6(SIGNAL_KILL, [0, pid as u64, tid as u64, signo as u64, code as u64, value as u64])
    }
    .map(|_| ())
}

/// A kernel call with six arguments.
///
/// # Safety
///
/// As [`kercall`].
unsafe fn kercall6(nr: u32, a: [u64; 6]) -> Result<u64, i32> {
    let (r, failed): (u64, u64);
    // SAFETY: the trap; the caller vouches for the arguments.
    unsafe {
        asm!(
            "mov x9, #0",
            "svc #0x51",
            "b 1f",
            "mov x9, #1",
            "1:",
            in("w8") nr,
            inlateout("x0") a[0] => r,
            in("x1") a[1], in("x2") a[2], in("x3") a[3], in("x4") a[4], in("x5") a[5],
            out("x9") failed,
            clobber_abi("C"),
        );
    }
    if failed == 0 { Ok(r) } else { Err(r as i32) }
}

/// OsCraft's calls for the FreeBSD personality (`kernel/src/personality/native.rs`).
const OSCRAFT_THR_NEW: u32 = 120;
const OSCRAFT_THR_SELF: u32 = 121;
const OSCRAFT_FUTEX: u32 = 122;
/// `__KER_TIMER_TIMEOUT`.
const TIMER_TIMEOUT: u32 = 75;
const THREAD_CTLEXT: u32 = 53;
const MSG_REGISTER_EVENT: u32 = 5;
const CONNECT_DUPFD: u32 = 44;

/// `ConnectDupFd(op, pid, handle, srvpid, info)`: a descriptor passed
/// to another process as a handle (`_NTO_DUPFD_*`).
///
/// # Safety
///
/// Each pointer is valid for the operation, or null where it allows.
pub unsafe fn connect_dupfd(op: u32, pid: *mut i64, handle: *mut u64, srvpid: *mut i32, info: *mut proto::info::MsgInfoHdr) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(CONNECT_DUPFD, [u64::from(op), pid as u64, handle as u64, srvpid as u64, info as u64]) }.map(|_| ())
}
const SCHED_YIELD: u32 = 90;
const TIMER_CREATE: u32 = 70;
const TIMER_DESTROY: u32 = 71;
const TIMER_SETTIME: u32 = 72;
const TIMER_INFO: u32 = 73;
const TIMER_ALARM: u32 = 74;

/// `SchedYield()`.
pub fn sched_yield() -> Result<(), i32> {
    // SAFETY: no arguments.
    unsafe { kercall(SCHED_YIELD, [0; 5]) }.map(|_| ())
}

/// `TimerCreate(clock, event)`: the timer's id.
///
/// # Safety
///
/// `event` is a `struct sigevent` or null.
pub unsafe fn timer_create(clock: i32, event: *const proto::event::Sigevent) -> Result<i32, i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(TIMER_CREATE, [clock as u64, event as u64, 0, 0, 0]) }.map(|id| id as i32)
}

/// `TimerDestroy(id)`.
pub fn timer_destroy(id: i32) -> Result<(), i32> {
    // SAFETY: no pointers.
    unsafe { kercall(TIMER_DESTROY, [id as u64, 0, 0, 0, 0]) }.map(|_| ())
}

/// `TimerSettime(id, flags, itime, oitime)`: `struct _itimer` is (nsec,
/// interval_nsec).
///
/// # Safety
///
/// `itime` is an `_itimer` or null, `oitime` a writable one or null.
pub unsafe fn timer_settime(id: i32, flags: u32, itime: *const [u64; 2], oitime: *mut [u64; 2]) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(TIMER_SETTIME, [id as u64, u64::from(flags), itime as u64, oitime as u64, 0]) }.map(|_| ())
}

/// `TimerInfo(pid, id, flags, info)`: `info` an 88-byte `struct
/// _timer_info`; the id.
///
/// # Safety
///
/// `info` is writable for 88 bytes.
pub unsafe fn timer_info(pid: i32, id: i32, flags: u32, info: *mut [u8; 88]) -> Result<i32, i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(TIMER_INFO, [pid as u64, id as u64, u64::from(flags), info as u64, 0]) }.map(|id| id as i32)
}

/// `TimerAlarm(clock, itime, otime)`: the process's alarm; a null
/// `itime` only reads.
///
/// # Safety
///
/// `itime` is an `_itimer` or null, `otime` a writable one or null.
pub unsafe fn timer_alarm(clock: i32, itime: *const [u64; 2], otime: *mut [u64; 2]) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(TIMER_ALARM, [clock as u64, itime as u64, otime as u64, 0, 0]) }.map(|_| ())
}
const SIGNAL_WAITINFO: u32 = 32;
/// `_NTO_TIMEOUT_SIGWAITINFO`.
pub const TIMEOUT_SIGWAITINFO: u32 = 1 << 11;

/// `MsgRegisterEvent(event, coid, &handle)`: registers `event` for the
/// server of `coid` to deliver (QNX 8 delivers only registered events);
/// with a null `event`, unregisters the handle.
///
/// # Safety
///
/// `event` is a `struct sigevent` or null; `handle` is writable.
pub unsafe fn msg_register_event(event: *const proto::event::Sigevent, coid: i32, handle: *mut u32) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(MSG_REGISTER_EVENT, [event as u64, coid as u64, handle as u64, 0, 0]) }.map(|_| ())
}

/// `SignalWaitinfoMask(set, info, mask)`: waits for a signal of `set`
/// (QNX's numbers), with the thread's mask `mask` meanwhile if given;
/// the signal's number, `info` (if given) its `siginfo_t`.
///
/// # Safety
///
/// `set` and `mask` are QNX signal sets (`mask` may be null); `info` is
/// a QNX `siginfo_t` or null.
pub unsafe fn signal_waitinfo(set: *const [u32; 4], info: *mut u8, mask: *const [u32; 4]) -> Result<i32, i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(SIGNAL_WAITINFO, [set as u64, info as u64, mask as u64, 0, 0]) }.map(|s| s as i32)
}
const SCHED_GET: u32 = 88;

/// `ThreadCtlExt(pid, tid, cmd, data)`: pid and tid 0 for the caller.
///
/// # Safety
///
/// `data` is what `cmd` takes (a value or a pointer the kernel may write).
pub unsafe fn thread_ctl_ext(pid: i32, tid: i32, cmd: u32, data: u64) -> Result<u64, i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(THREAD_CTLEXT, [pid as u64, tid as u64, u64::from(cmd), data, 0]) }
}
const SCHED_SET: u32 = 89;

/// `SchedGet(pid, tid, param)`: the thread's policy; `param` (a QNX
/// `sched_param`, 48 bytes) takes its base and current priorities.
pub fn sched_get(pid: i32, tid: i32, param: &mut [u8; 48]) -> Result<u32, i32> {
    // SAFETY: `param` is a buffer of the call's size.
    unsafe { kercall(SCHED_GET, [pid as u64, tid as u64, param.as_mut_ptr() as u64, 0, 0]) }.map(|v| v as u32)
}

/// `SchedSet(pid, tid, policy, param)`.
pub fn sched_set(pid: i32, tid: i32, policy: u32, param: &[u8; 48]) -> Result<(), i32> {
    // SAFETY: `param` is a buffer of the call's size.
    unsafe { kercall(SCHED_SET, [pid as u64, tid as u64, u64::from(policy), param.as_ptr() as u64, 0]) }.map(|_| ())
}
/// `_NTO_TIMEOUT_SEM`: the state a futex wait blocks in.
pub const TIMEOUT_SEM: u32 = 1 << 14;
/// `TIMER_ABSTIME` in `TimerTimeout()`'s flags: the time is absolute.
pub const TIMEOUT_ABSTIME: u32 = 0x8000_0000;

/// `thr_new(param, size)`, FreeBSD's, in the kernel.
///
/// # Safety
///
/// `param` points to a `struct thr_param`.
pub unsafe fn thr_new(param: *const c_void, size: usize, sched: Option<(u32, u32)>) -> Result<(), i32> {
    // The thread's policy and priority, if the caller names them (0: its own).
    let (policy, priority) = sched.unwrap_or((0, 0));
    // SAFETY: the caller's guarantee.
    unsafe { kercall(OSCRAFT_THR_NEW, [param as u64, size as u64, u64::from(policy), u64::from(priority), 0]) }.map(|_| ())
}

/// The calling thread's ID.
/// A FreeBSD thread ID as the kernel's calls take it: FreeBSD's start at
/// `TID_BASE`, above `PID_MAX` (`personality/native.rs`).
pub fn kernel_tid(id: i64) -> i32 {
    const TID_BASE: i64 = 100_000;
    if id >= TID_BASE { (id - TID_BASE) as i32 } else { id as i32 }
}

pub fn thr_self() -> Result<i64, i32> {
    // SAFETY: no pointers.
    unsafe { kercall(OSCRAFT_THR_SELF, [0; 5]) }.map(|t| t as i64)
}

/// Waits while the 32-bit word at `addr` holds `val`: Ok when woken,
/// EAGAIN if it did not, ETIMEDOUT or EINTR.
///
/// # Safety
///
/// `addr` is an aligned 32-bit word of the process.
pub unsafe fn futex_wait(addr: *const u32, val: u32) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(OSCRAFT_FUTEX, [addr as u64, 0, u64::from(val), 0, 0]) }.map(|_| ())
}

/// A wait that raises thread `owner`, the holder of the mutex at `addr`,
/// to the waiter's priority meanwhile (FreeBSD's PTHREAD_PRIO_INHERIT).
///
/// # Safety
///
/// As [`futex_wait`].
pub unsafe fn futex_wait_pi(addr: *const u32, val: u32, owner: u32) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(OSCRAFT_FUTEX, [addr as u64, 2, u64::from(val), u64::from(owner), 0]) }.map(|_| ())
}

/// Wakes up to `n` waiters on the word at `addr`: how many woke, and how
/// many still wait.
pub fn futex_wake(addr: *const u32, n: u32) -> (u32, u32) {
    // SAFETY: no memory is read.
    unsafe { kercall(OSCRAFT_FUTEX, [addr as u64, 1, u64::from(n), 0, 0]) }
        .map_or((0, 0), |w| (w as u32, (w >> 32) as u32))
}

/// `TimerTimeout(clock, flags, NULL, &ns, NULL)`: arms a timeout for the
/// next blocking call in a state `flags` names.
pub fn timer_timeout(clock: i32, flags: u32, ns: u64) -> Result<(), i32> {
    // SAFETY: `ns` is on this stack for the call.
    unsafe { kercall(TIMER_TIMEOUT, [clock as u64, u64::from(flags), 0, (&raw const ns) as u64, 0]) }
        .map(|_| ())
}

/// `__KER_SIGNAL_ACTION` and `__KER_SIGNAL_RETURN`.
const SIGNAL_ACTION: u32 = 29;
const SIGNAL_RETURN: u32 = 27;

/// `SignalAction(0, stub, signo, act, oact)`: QNX's `struct sigaction`s.
///
/// # Safety
///
/// `act` and `oact` are QNX sigactions or null.
pub unsafe fn signal_action(stub: u64, signo: i32, act: *const crate::sigaction::Sigaction, oact: *mut crate::sigaction::Sigaction) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(SIGNAL_ACTION, [0, stub, signo as u64, act as u64, oact as u64]) }.map(|_| ())
}

/// `SignalReturn(frame)`: back to the context a handler interrupted; it
/// does not return.
///
/// # Safety
///
/// `frame` is the frame the kernel gave the trampoline.
pub unsafe fn signal_return(frame: *mut u8) -> ! {
    loop {
        // SAFETY: the caller's guarantee.
        let _ = unsafe { kercall(SIGNAL_RETURN, [frame as u64, 0, 0, 0, 0]) };
    }
}

/// `__KER_CONNECT_SERVER_INFO`, `__KER_CONNECT_FLAGS`, `__KER_SIGNAL_SUSPEND`.
const CONNECT_SERVER_INFO: u32 = 41;
const CONNECT_FLAGS: u32 = 43;
const SIGNAL_SUSPEND: u32 = 31;
/// `_NTO_TIMEOUT_NANOSLEEP` (`1 << STATE_NANOSLEEP`, 12): a sleep in the
/// call itself.
pub const TIMEOUT_NANOSLEEP: u32 = 1 << 12;

/// `ConnectServerInfo(0, coid, info)`: the connection's server, into
/// `info` (a `struct _server_info`); the connection ID back.
///
/// # Safety
///
/// `info` is a `struct _server_info`.
pub unsafe fn connect_server_info(coid: i32, info: *mut c_void) -> Result<i32, i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(CONNECT_SERVER_INFO, [0, coid as u64, info as u64, 0, 0]) }.map(|c| c as i32)
}

/// `ConnectFlags(0, coid, mask, bits)`: sets the connection's flags in
/// `mask` to `bits`; the old flags.
pub fn connect_flags(coid: i32, mask: u32, bits: u32) -> Result<u32, i32> {
    // SAFETY: no pointers.
    unsafe { kercall(CONNECT_FLAGS, [0, coid as u64, u64::from(mask), u64::from(bits), 0]) }.map(|f| f as u32)
}

/// `SignalSuspend(set)`.
///
/// # Safety
///
/// `set` is a QNX set.
pub unsafe fn signal_suspend(set: *const [u32; 4]) -> Result<(), i32> {
    // SAFETY: the caller's guarantee.
    unsafe { kercall(SIGNAL_SUSPEND, [set as u64, 0, 0, 0, 0]) }.map(|_| ())
}

/// `TimerTimeout(clock, flags, NULL, &ns, left)`: with
/// `_NTO_TIMEOUT_NANOSLEEP`, sleeps; `left` gets the time left if a
/// signal ended it.
///
/// # Safety
///
/// `left` points to a u64.
pub unsafe fn timer_timeout_left(clock: i32, flags: u32, ns: u64, left: *mut u64) -> Result<(), i32> {
    // SAFETY: `ns` is on this stack; `left` the caller's.
    unsafe { kercall(TIMER_TIMEOUT, [clock as u64, u64::from(flags), 0, (&raw const ns) as u64, left as u64]) }
        .map(|_| ())
}

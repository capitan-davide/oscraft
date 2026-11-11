//! Signals.

use core::ffi::*;

use super::nr;
use crate::abi::*;

kercall! {
    /// `int SignalAction(pid_t __pid, void (*__sigstub)(void), int __signo, const struct sigaction *__act, struct sigaction *__oact)`
    errno SignalAction = nr::SIGNAL_ACTION;
    fn(pid: PidT, sigstub: *const c_void, signo: c_int, act: *const c_void, oact: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalAction_r(pid_t __pid, void (*__sigstub)(void), int __signo, const struct sigaction *__act, struct sigaction *__oact)`
    status SignalAction_r = nr::SIGNAL_ACTION;
    fn(pid: PidT, sigstub: *const c_void, signo: c_int, act: *const c_void, oact: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalKill(_Uint32t __nd, pid_t __pid, int __tid, int __signo, int __code, int __value)`
    errno SignalKill = nr::SIGNAL_KILL;
    fn(nd: u32, pid: PidT, tid: c_int, signo: c_int, code: c_int, value: c_int) -> c_int
}

kercall! {
    /// `int SignalKill_r(_Uint32t __nd, pid_t __pid, int __tid, int __signo, int __code, int __value)`
    status SignalKill_r = nr::SIGNAL_KILL;
    fn(nd: u32, pid: PidT, tid: c_int, signo: c_int, code: c_int, value: c_int) -> c_int
}

kercall! {
    /// `int SignalKillSigval(_Uint32t __nd, pid_t __pid, int __tid, int __signo, int __code, const union sigval *__value)`
    errno SignalKillSigval = nr::SIGNAL_KILL_SIGVAL;
    fn(nd: u32, pid: PidT, tid: c_int, signo: c_int, code: c_int, value: *const c_void) -> c_int
}

kercall! {
    /// `int SignalKillSigval_r(_Uint32t __nd, pid_t __pid, int __tid, int __signo, int __code, const union sigval *__value)`
    status SignalKillSigval_r = nr::SIGNAL_KILL_SIGVAL;
    fn(nd: u32, pid: PidT, tid: c_int, signo: c_int, code: c_int, value: *const c_void) -> c_int
}

kercall! {
    /// `int SignalProcmask(pid_t __pid, int __tid, int __how, const sigset_t *__set, sigset_t *__oldset)`
    errno SignalProcmask = nr::SIGNAL_PROCMASK;
    fn(pid: PidT, tid: c_int, how: c_int, set: *const c_void, oldset: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalProcmask_r(pid_t __pid, int __tid, int __how, const sigset_t *__set, sigset_t *__oldset)`
    status SignalProcmask_r = nr::SIGNAL_PROCMASK;
    fn(pid: PidT, tid: c_int, how: c_int, set: *const c_void, oldset: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalReturn(struct _sighandler_info *__info)`
    errno SignalReturn = nr::SIGNAL_RETURN;
    fn(info: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalReturn_r(struct _sighandler_info *__info)`
    neg SignalReturn_r = nr::SIGNAL_RETURN;
    fn(info: *mut c_void) -> c_int
}

kercall! {
    /// `int SignalSuspend(const sigset_t *__set)`
    errno SignalSuspend = nr::SIGNAL_SUSPEND;
    fn(set: *const c_void) -> c_int
}

kercall! {
    /// `int SignalSuspend_r(const sigset_t *__set)`
    status SignalSuspend_r = nr::SIGNAL_SUSPEND;
    fn(set: *const c_void) -> c_int
}

kercall! {
    /// `int SignalWaitinfoMask(const sigset_t *__set, siginfo_t *__info, const sigset_t *__mask)`
    errno SignalWaitinfoMask = nr::SIGNAL_WAITINFO;
    fn(set: *const c_void, info: *mut c_void, mask: *const c_void) -> c_int
}

kercall! {
    /// `int SignalWaitinfoMask_r(const sigset_t *__set, siginfo_t *__info, const sigset_t *__mask)`
    neg SignalWaitinfoMask_r = nr::SIGNAL_WAITINFO;
    fn(set: *const c_void, info: *mut c_void, mask: *const c_void) -> c_int
}

// Functions built on the calls above.

/// `int SignalWaitinfo(const sigset_t *set, siginfo_t *info)`: waits for a
/// signal in `set`, without changing the signal mask.
#[unsafe(no_mangle)]
pub extern "C" fn SignalWaitinfo(set: *const c_void, info: *mut c_void) -> c_int {
    SignalWaitinfoMask(set, info, core::ptr::null())
}

/// `SignalWaitinfo()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn SignalWaitinfo_r(set: *const c_void, info: *mut c_void) -> c_int {
    SignalWaitinfoMask_r(set, info, core::ptr::null())
}

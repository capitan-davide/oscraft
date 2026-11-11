//! Signals: FreeBSD's numbers and sets are the kernel's, so these calls
//! pass them through.

use core::ffi::c_int;

use crate::errno::fail;
use crate::kercall::{signal_kill, signal_procmask};

/// FreeBSD's `sigset_t`.
pub type SigSet = [u32; 4];
/// `sigprocmask(how, set, oset)`: `SignalProcmask()` on the calling
/// thread.
///
/// # Safety
///
/// `set` and `oset` are FreeBSD sets or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigprocmask(how: c_int, set: *const SigSet, oset: *mut SigSet) -> c_int {
    // SAFETY: the caller's guarantee.
    match unsafe { signal_procmask(how, set, oset) } {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `kill(pid, sig)`: `SignalKill()`; `sig` 0 checks that the process
/// exists.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_kill(pid: c_int, sig: c_int) -> c_int {
    const SI_USER: c_int = 0;
    match signal_kill(pid, 0, sig, SI_USER, 0) {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `sigqueue(pid, signo, value)`: `SignalKill()` with `SI_QUEUE` and the
/// value (QNX keeps the low 32 bits of a `sigval`, as a `sival_int`).
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sigqueue(pid: c_int, signo: c_int, value: u64) -> c_int {
    const SI_QUEUE: c_int = 0x10002;
    match crate::kercall::signal_kill(pid, 0, signo, SI_QUEUE, value as i32) {
        Ok(()) => 0,
        Err(e) => crate::errno::fail(e) as c_int,
    }
}

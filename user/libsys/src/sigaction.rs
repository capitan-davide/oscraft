//! `sigaction()`, and the trampoline a handler runs through.
//!
//! The kernel saves the thread's context on its stack as FreeBSD's
//! `ucontext_t`, puts a frame below it (FreeBSD's `siginfo_t`, the
//! handler, the context's address) and starts the trampoline `sigaction()`
//! registered, with x0 at the frame. The trampoline
//! ([`__oscraft_sigstub`]) calls the handler and ends with
//! `SignalReturn()`, which takes the context back as the handler left it.

use core::ffi::{c_int, c_void};

use proto::errno::{EAGAIN, EINVAL, ETIMEDOUT};

use crate::errno::fail;
use crate::kercall::{self, signal_action, signal_return};
use crate::signal::SigSet;

/// FreeBSD's `struct sigaction`: the handler, `sa_flags`, `sa_mask`.
#[repr(C)]
pub struct Sigaction {
    pub handler: u64,
    pub flags: c_int,
    pub mask: SigSet,
}

/// `sigaction(sig, act, oact)`: `SignalAction()` with the trampoline.
///
/// # Safety
///
/// `act` and `oact` are FreeBSD sigactions or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigaction(sig: c_int, act: *const Sigaction, oact: *mut Sigaction) -> c_int {
    // SAFETY: the caller's guarantee.
    match unsafe { signal_action(__oscraft_sigstub as *const () as usize as u64, sig, act, oact) } {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

const FRAME_HANDLER: usize = 80;
const FRAME_CONTEXT: usize = 88;

// ANCHOR: sigstub
/// The trampoline.
///
/// # Safety
///
/// Only the kernel starts it, with `frame` as described above.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __oscraft_sigstub(frame: *mut u8) -> ! {
    // SAFETY: the kernel's frame.
    let (signo, handler, context) = unsafe {
        (
            frame.cast::<c_int>().read(),
            frame.add(FRAME_HANDLER).cast::<u64>().read(),
            frame.add(FRAME_CONTEXT).cast::<*mut c_void>().read(),
        )
    };
    let handler: extern "C" fn(c_int, *mut u8, *mut c_void) =
        // SAFETY: a handler the program gave sigaction(); a plain
        // `sa_handler` ignores the extra arguments.
        unsafe { core::mem::transmute::<u64, extern "C" fn(c_int, *mut u8, *mut c_void)>(handler) };
    handler(signo, frame, context);
    // SAFETY: the frame is the kernel's.
    unsafe { signal_return(frame) }
}
// ANCHOR_END: sigstub

/// `sigpending(set)`: `SignalProcmask(SIG_PENDING)` reads the pending
/// blocked signals.
///
/// # Safety
///
/// `set` is a FreeBSD set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigpending(set: *mut SigSet) -> c_int {
    /// One past FreeBSD's `how`s, where QNX's `SIG_PENDING` sits.
    const SIG_PENDING: i32 = 5;
    // SAFETY: the caller's guarantee.
    match unsafe { crate::kercall::signal_procmask(SIG_PENDING, core::ptr::null(), set) } {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `sigtimedwait(set, info, timeout)`: `SignalWaitinfo()`, with
/// `TimerTimeout()` as the limit (EAGAIN when it passes, as FreeBSD says).
///
/// # Safety
///
/// `set` is a FreeBSD set; `info` a `siginfo_t` or null; `timeout` a
/// timespec or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigtimedwait(set: *const SigSet, info: *mut u8, timeout: *const [i64; 2]) -> c_int {
    if !timeout.is_null() {
        // SAFETY: the caller's guarantee.
        let [s, n] = unsafe { *timeout };
        if s < 0 || !(0..1_000_000_000).contains(&n) {
            return fail(EINVAL) as c_int;
        }
        let _ = kercall::timer_timeout(0, kercall::TIMEOUT_SIGWAITINFO, s as u64 * 1_000_000_000 + n as u64);
    }
    // SAFETY: as above; no mask.
    match unsafe { kercall::signal_waitinfo(set, info, core::ptr::null()) } {
        Ok(signo) => signo,
        Err(ETIMEDOUT) => fail(EAGAIN) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

/// `sigwaitinfo(set, info)`: `sigtimedwait()` without a limit.
///
/// # Safety
///
/// As `sigtimedwait()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigwaitinfo(set: *const SigSet, info: *mut u8) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_sigtimedwait(set, info, core::ptr::null()) }
}

/// `sigwait(set, sig)`: the error number as the result (FreeBSD's
/// `sys_sigwait` returns it so), 0 with the signal in `*sig`.
///
/// # Safety
///
/// `set` is a FreeBSD set; `sig` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigwait(set: *const SigSet, sig: *mut c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let r = unsafe { __sys_sigtimedwait(set, core::ptr::null_mut(), core::ptr::null()) };
    if r == -1 {
        // SAFETY: __error() is the thread's errno.
        return unsafe { *crate::errno::__error() };
    }
    // SAFETY: the caller's guarantee.
    unsafe { sig.write(r) };
    0
}

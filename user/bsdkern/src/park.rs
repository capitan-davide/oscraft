//! Where a sleeping thread waits: `sched_oscraft_switch()` in
//! `kern/sched.c`. A wake-up that comes first is kept, since the thread lets
//! go of the sleep chain's lock before it parks.

use core::ffi::c_void;
use std::sync::{Condvar, Mutex};

#[derive(Default)]
struct Parker {
    woken: Mutex<bool>,
    wake: Condvar,
}

/// A parker for one thread.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_parker_new() -> *mut c_void {
    Box::into_raw(Box::<Parker>::default()).cast()
}

/// Waits for `bsdkern_unpark()`, or returns at once for one already made.
///
/// # Safety
///
/// `p` came from `bsdkern_parker_new()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_park(p: *mut c_void) {
    // SAFETY: the caller's guarantee; parkers are never freed while used.
    let p = unsafe { &*p.cast::<Parker>() };
    let mut woken = p.woken.lock().unwrap();
    while !*woken {
        woken = p.wake.wait(woken).unwrap();
    }
    *woken = false;
}

/// Wakes the thread parked on `p`, now or at its next park.
///
/// # Safety
///
/// As `bsdkern_park()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_unpark(p: *mut c_void) {
    // SAFETY: the caller's guarantee.
    let p = unsafe { &*p.cast::<Parker>() };
    *p.woken.lock().unwrap() = true;
    p.wake.notify_one();
}

/// `sched_relinquish()`.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_yield() {
    std::thread::yield_now();
}

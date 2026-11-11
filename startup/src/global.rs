//! Startup's global state.
//!
//! Startup is a single-threaded program until the secondary CPUs start,
//! and those only touch the state they are handed (the code that starts
//! them uses atomics). So its few globals need no locks; [`Global`] only
//! lets them be `static`, and each access states why it is safe.

use core::cell::UnsafeCell;

/// A value that startup's boot CPU owns.
pub struct Global<T>(UnsafeCell<T>);

// SAFETY: see the module documentation; every access is `unsafe` and
// justified at the call site.
unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    /// A pointer to the value.
    ///
    /// # Safety
    ///
    /// No other reference to the value may be live while the pointer is
    /// used.
    pub const unsafe fn get(&self) -> *mut T {
        self.0.get()
    }
}

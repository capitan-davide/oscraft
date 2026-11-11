//! Synchronisation primitives.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU32, Ordering};

// ANCHOR: spin_lock
/// A ticket spinlock (FIFO-fair).
///
/// Does not mask interrupts: code that also runs in interrupt context must
/// not take it from thread context with interrupts enabled.
pub struct SpinLock<T> {
    next: AtomicU32,
    serving: AtomicU32,
    value: UnsafeCell<T>,
}

// SAFETY: access to `value` is serialised by the lock.
unsafe impl<T: Send> Sync for SpinLock<T> {}
// SAFETY: the lock owns its value; sending it sends the value.
unsafe impl<T: Send> Send for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            next: AtomicU32::new(0),
            serving: AtomicU32::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinGuard<'_, T> {
        let ticket = self.next.fetch_add(1, Ordering::Relaxed);
        while self.serving.load(Ordering::Acquire) != ticket {
            core::hint::spin_loop();
        }
        SpinGuard { lock: self }
    }
}
// ANCHOR_END: spin_lock

pub struct SpinGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

/// A spinlock that also masks IRQs on this CPU while held, so it can be
/// shared between thread and interrupt context.
pub struct IrqSpinLock<T> {
    inner: SpinLock<T>,
}

impl<T> IrqSpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: SpinLock::new(value),
        }
    }

    pub fn lock(&self) -> IrqSpinGuard<'_, T> {
        let daif = crate::arch::irq_save();
        IrqSpinGuard {
            guard: Some(self.inner.lock()),
            daif,
        }
    }
}

pub struct IrqSpinGuard<'a, T> {
    guard: Option<SpinGuard<'a, T>>,
    daif: u64,
}

impl<T> Deref for IrqSpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.as_ref().unwrap()
    }
}

impl<T> DerefMut for IrqSpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.as_mut().unwrap()
    }
}

// ANCHOR: irq_guard_drop
impl<T> Drop for IrqSpinGuard<'_, T> {
    fn drop(&mut self) {
        // Release the lock before unmasking interrupts.
        drop(self.guard.take());
        crate::arch::irq_restore(self.daif);
    }
}
// ANCHOR_END: irq_guard_drop

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: the guard holds the lock.
        unsafe { &*self.lock.value.get() }
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: the guard holds the lock exclusively.
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.serving.fetch_add(1, Ordering::Release);
    }
}

/// A value set once (during boot) and then shared read-only, also with
/// interrupt handlers (no lock is taken to read it).
pub struct InitOnce<T> {
    state: core::sync::atomic::AtomicU8,
    value: UnsafeCell<core::mem::MaybeUninit<T>>,
}

const EMPTY: u8 = 0;
const BUSY: u8 = 1;
const READY: u8 = 2;

// SAFETY: the value is written once before READY is published and only
// read afterwards.
unsafe impl<T: Send + Sync> Sync for InitOnce<T> {}

impl<T> InitOnce<T> {
    pub const fn new() -> Self {
        Self {
            state: core::sync::atomic::AtomicU8::new(EMPTY),
            value: UnsafeCell::new(core::mem::MaybeUninit::uninit()),
        }
    }

    /// Set the value; fails (returning it) if already set.
    pub fn set(&self, value: T) -> Result<(), T> {
        if self
            .state
            .compare_exchange(EMPTY, BUSY, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return Err(value);
        }
        // SAFETY: BUSY gives this caller exclusive access.
        unsafe { (*self.value.get()).write(value) };
        self.state.store(READY, Ordering::Release);
        Ok(())
    }

    pub fn get(&self) -> Option<&T> {
        // SAFETY: READY means the value is initialised and no longer written.
        (self.state.load(Ordering::Acquire) == READY)
            .then(|| unsafe { &*self.value.get().cast::<T>() })
    }
}

impl<T> Drop for InitOnce<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == READY {
            // SAFETY: READY means the value is initialised; nobody uses it
            // any more.
            unsafe { self.value.get_mut().assume_init_drop() };
        }
    }
}

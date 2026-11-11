//! FreeBSD's `mtx`, `rw` and `sx`, built with `MUTEX_NOINLINE`,
//! `RWLOCK_NOINLINE` and `SX_NOINLINE`, so each operation is one of these
//! functions and the structures stay FreeBSD's. `io-sock` has the same
//! shape (no turnstiles; `_mtx_init` makes a `pthread_mutex_t`,
//! `_sx_xlock_hard` a `pthread_rwlock_t` with an owner and a depth) but
//! changes `struct mtx`. Here the lock proper hangs from `lo_witness`,
//! unused without `WITNESS`, and the word after the `lock_object` keeps
//! FreeBSD's meaning (the owning thread), so that `mtx_owned()`,
//! `sx_xlocked()` and the assertions work unchanged.
//!
//! `rw` and `sx` need upgrades and atomic downgrades, which pthreads don't
//! have (`io-sock` reaches into QNX's rwlock): `Shared` is a small
//! reader-writer lock of its own.

use core::ffi::{c_char, c_int, c_uint, c_void};
use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};

/// `struct lock_object`, which the word follows.
#[repr(C)]
pub struct LockObject {
    name: *const c_char,
    flags: c_uint,
    data: c_uint,
    witness: *mut c_void,
}

const COOKIE: usize = size_of::<LockObject>();
const LO_INITIALIZED: u32 = 0x1_0000;
const LO_CLASSSHIFT: u32 = 24;
/// Positions in `lock_classes[]`, `kern/subr_lock.c`.
const CLASS_MTX_SPIN: u32 = 0;
const CLASS_MTX_SLEEP: u32 = 1;
const CLASS_SX: u32 = 2;
const CLASS_RW: u32 = 5;
const MTX_SPIN: c_int = 1;
const MTX_RECURSE: c_int = 4;
/// `RW_UNLOCKED`, `SX_LOCK_UNLOCKED`: no writer.
const NO_WRITER: usize = 1;
const LC_SLEEPLOCK: u32 = 1;
const LC_SPINLOCK: u32 = 2;
const LC_SLEEPABLE: u32 = 4;
const LC_RECURSABLE: u32 = 8;
const LC_UPGRADABLE: u32 = 16;
const EBUSY: c_int = 16;

type Pthread = *mut c_void;

unsafe extern "C" {
    fn pthread_mutexattr_init(a: *mut Pthread) -> c_int;
    fn pthread_mutexattr_settype(a: *mut Pthread, t: c_int) -> c_int;
    fn pthread_mutexattr_setprotocol(a: *mut Pthread, p: c_int) -> c_int;
    fn pthread_mutexattr_destroy(a: *mut Pthread) -> c_int;
    fn pthread_mutex_init(m: *mut Pthread, a: *const Pthread) -> c_int;
    fn pthread_mutex_destroy(m: *mut Pthread) -> c_int;
    fn pthread_mutex_lock(m: *mut Pthread) -> c_int;
    fn pthread_mutex_trylock(m: *mut Pthread) -> c_int;
    fn pthread_mutex_unlock(m: *mut Pthread) -> c_int;
    fn bsdkern_get_curthread() -> *mut c_void;
}

fn me() -> usize {
    // SAFETY: reads a thread-local pointer.
    unsafe { bsdkern_get_curthread() as usize }
}

/// The `lock_object` before a lock's word.
///
/// # Safety
///
/// `c` is the address of a lock's word.
unsafe fn object(c: *const usize) -> *mut LockObject {
    // SAFETY: the word follows the object, in one structure.
    unsafe { c.cast::<u8>().sub(COOKIE).cast::<LockObject>().cast_mut() }
}

/// The word, read and written by FreeBSD's macros too.
///
/// # Safety
///
/// `c` is the address of a lock's word.
unsafe fn word<'a>(c: *const usize) -> &'a AtomicUsize {
    // SAFETY: aligned, and lives as long as the lock.
    unsafe { AtomicUsize::from_ptr(c.cast_mut()) }
}

// mtx

/// `mtx_init()`. `opts` as FreeBSD's; `MTX_RECURSE` makes it recursive as
/// in `_mtx_init`, and every one inherits priority, unlike `io-sock`'s.
///
/// # Safety
///
/// `c` is a `struct mtx`'s word.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mtx_init(c: *mut usize, name: *const c_char, _kind: *const c_char, opts: c_int) {
    const RECURSIVE: c_int = 2;
    const PRIO_INHERIT: c_int = 1;
    let m = Box::into_raw(Box::new(null_mut::<c_void>()));
    let mut attr = null_mut();
    // SAFETY: `attr` and `m` are valid for these calls.
    unsafe {
        assert_eq!(pthread_mutexattr_init(&raw mut attr), 0);
        if opts & MTX_RECURSE != 0 {
            assert_eq!(pthread_mutexattr_settype(&raw mut attr, RECURSIVE), 0);
        }
        assert_eq!(pthread_mutexattr_setprotocol(&raw mut attr, PRIO_INHERIT), 0);
        assert_eq!(pthread_mutex_init(m, &raw const attr), 0);
        pthread_mutexattr_destroy(&raw mut attr);
        let class = if opts & MTX_SPIN != 0 { CLASS_MTX_SPIN } else { CLASS_MTX_SLEEP };
        object(c).write(LockObject {
            name,
            flags: class << LO_CLASSSHIFT | LO_INITIALIZED,
            data: 0,
            witness: m.cast(),
        });
        c.write(0);
    }
}

/// `mtx_destroy()`.
///
/// # Safety
///
/// As `_mtx_init()`; the lock is free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mtx_destroy(c: *mut usize) {
    // SAFETY: the caller's guarantee; `_mtx_init` made it.
    unsafe {
        let m = (*object(c)).witness.cast::<Pthread>();
        pthread_mutex_destroy(m);
        drop(Box::from_raw(m));
        (*object(c)).witness = null_mut();
    }
}

/// # Safety
///
/// `c` is an initialised `struct mtx`'s word.
unsafe fn pthread_of(c: *const usize) -> *mut Pthread {
    // SAFETY: the caller's guarantee.
    unsafe { (*object(c)).witness.cast() }
}

/// The lock just taken: the owner, or one more level.
///
/// # Safety
///
/// As `pthread_of()`, and held.
unsafe fn taken(c: *const usize) {
    // SAFETY: held, so only this thread writes.
    unsafe {
        if word(c).load(Ordering::Relaxed) == me() {
            (*object(c)).data += 1;
        } else {
            word(c).store(me(), Ordering::Relaxed);
        }
    }
}

/// `mtx_lock()`.
///
/// # Safety
///
/// As `pthread_of()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mtx_lock_flags(c: *mut usize, _o: c_int, _f: *const c_char, _l: c_int) {
    // SAFETY: the caller's guarantee.
    unsafe {
        assert_eq!(pthread_mutex_lock(pthread_of(c)), 0);
        taken(c);
    }
}

/// `mtx_unlock()`.
///
/// # Safety
///
/// As `pthread_of()`, held by this thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mtx_unlock_flags(c: *mut usize, _o: c_int, _f: *const c_char, _l: c_int) {
    // SAFETY: the caller's guarantee.
    unsafe {
        let o = object(c);
        if (*o).data > 0 {
            (*o).data -= 1;
        } else {
            word(c).store(0, Ordering::Relaxed);
        }
        assert_eq!(pthread_mutex_unlock(pthread_of(c)), 0);
    }
}

/// `mtx_trylock()`: 1 if taken.
///
/// # Safety
///
/// As `pthread_of()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _mtx_trylock_flags_(c: *mut usize, _o: c_int, _f: *const c_char, _l: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe {
        match pthread_mutex_trylock(pthread_of(c)) {
            0 => {
                taken(c);
                1
            }
            EBUSY => 0,
            e => panic!("bsdkern: mtx_trylock: {e}"),
        }
    }
}

/// `mtx_lock_spin()`: a mutex too, as `io-sock`'s `__mtx_lock_spin`, and
/// inside a critical section, as FreeBSD's after `spinlock_enter()` (which
/// `io-sock` makes `__critical_enter`, read with REA). So a CPU slot always
/// comes before a spin lock: the clock holds a slot and wants a callout
/// wheel's lock, and a holder of that lock needing the slot would be a
/// deadlock.
///
/// # Safety
///
/// As `__mtx_lock_flags()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mtx_lock_spin_flags(c: *mut usize, o: c_int, f: *const c_char, l: c_int) {
    crate::slot::critical_enter_KBI();
    // SAFETY: the caller's guarantee.
    unsafe { __mtx_lock_flags(c, o, f, l) }
}

/// `mtx_trylock_spin()`.
///
/// # Safety
///
/// As `_mtx_trylock_flags_()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mtx_trylock_spin_flags(c: *mut usize, o: c_int, f: *const c_char, l: c_int) -> c_int {
    crate::slot::critical_enter_KBI();
    // SAFETY: the caller's guarantee.
    let got = unsafe { _mtx_trylock_flags_(c, o, f, l) };
    if got == 0 {
        crate::slot::critical_exit_KBI();
    }
    got
}

/// `mtx_unlock_spin()`.
///
/// # Safety
///
/// As `__mtx_unlock_flags()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __mtx_unlock_spin_flags(c: *mut usize, o: c_int, f: *const c_char, l: c_int) {
    // SAFETY: the caller's guarantee.
    unsafe { __mtx_unlock_flags(c, o, f, l) }
    crate::slot::critical_exit_KBI();
}

/// `struct mtx_args`, `struct rw_args`, `struct sx_args`.
#[repr(C)]
pub struct InitArgs {
    lock: *mut u8,
    name: *const c_char,
    opts: c_int,
}

/// `MTX_SYSINIT()`.
///
/// # Safety
///
/// `a` names a `struct mtx`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mtx_sysinit(a: *const InitArgs) {
    // SAFETY: the caller's guarantee.
    unsafe { _mtx_init((*a).lock.add(COOKIE).cast(), (*a).name, null_mut(), (*a).opts) }
}

/// `struct mtx`, for `Giant`: the lock, and FreeBSD's name for it.
#[repr(C)]
pub struct Mtx {
    object: LockObject,
    word: usize,
}

/// FreeBSD's `Giant`; `init()` makes it.
#[unsafe(no_mangle)]
pub static mut Giant: Mtx = Mtx {
    object: LockObject { name: core::ptr::null(), flags: 0, data: 0, witness: null_mut() },
    word: 0,
};

/// Makes `Giant`; call once, before any lock is used.
pub fn init() {
    // SAFETY: nothing else runs yet.
    unsafe {
        let w = &raw mut Giant.word;
        _mtx_init(w, c"Giant".as_ptr(), null_mut(), MTX_RECURSE);
    }
}

// rw and sx

/// A reader-writer lock: `state` counts readers, or is `WRITER`. The fast
/// paths are atomic; waiters sleep on the mutex and condition.
struct Shared {
    state: AtomicU32,
    waiters: AtomicU32,
    gate: StdMutex<()>,
    freed: Condvar,
}

const WRITER: u32 = u32::MAX;

impl Shared {
    fn new() -> Self {
        Self { state: AtomicU32::new(0), waiters: AtomicU32::new(0), gate: StdMutex::new(()), freed: Condvar::new() }
    }

    fn try_read(&self) -> bool {
        let mut s = self.state.load(Ordering::Relaxed);
        while s != WRITER {
            match self.state.compare_exchange_weak(s, s + 1, Ordering::Acquire, Ordering::Relaxed) {
                Ok(_) => return true,
                Err(now) => s = now,
            }
        }
        false
    }

    fn try_write(&self) -> bool {
        self.state.compare_exchange(0, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    fn wait_until(&self, take: impl Fn(&Self) -> bool) {
        if take(self) {
            return;
        }
        let mut gate = self.gate.lock().unwrap();
        self.waiters.fetch_add(1, Ordering::SeqCst);
        while !take(self) {
            gate = self.freed.wait(gate).unwrap();
        }
        self.waiters.fetch_sub(1, Ordering::SeqCst);
    }

    fn wake(&self) {
        if self.waiters.load(Ordering::SeqCst) != 0 {
            drop(self.gate.lock().unwrap());
            self.freed.notify_all();
        }
    }

    fn unlock_read(&self) {
        if self.state.fetch_sub(1, Ordering::Release) == 1 {
            self.wake();
        }
    }

    fn unlock_write(&self) {
        self.state.store(0, Ordering::Release);
        self.wake();
    }

    /// The only reader becomes the writer.
    fn try_upgrade(&self) -> bool {
        self.state.compare_exchange(1, WRITER, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }

    /// The writer becomes a reader, with no gap for another writer.
    fn downgrade(&self) {
        self.state.store(1, Ordering::Release);
        self.wake();
    }
}

/// # Safety
///
/// `c` is an initialised `rw` or `sx`'s word.
unsafe fn shared<'a>(c: *const usize) -> &'a Shared {
    // SAFETY: the caller's guarantee. A lock of a part of the kernel left out
    // (`kern/stubs.c`: zeroed room) is made at first use.
    unsafe {
        let slot = AtomicPtr::from_ptr(&raw mut (*object(c)).witness);
        let mut lock = slot.load(Ordering::Acquire);
        if lock.is_null() {
            let new: *mut c_void = Box::into_raw(Box::new(Shared::new())).cast();
            lock = match slot.compare_exchange(null_mut(), new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => new,
                Err(now) => {
                    drop(Box::from_raw(new.cast::<Shared>()));
                    now
                }
            };
        }
        &*lock.cast::<Shared>()
    }
}

/// # Safety
///
/// `c` is an `rw` or `sx`'s word.
unsafe fn shared_init(c: *mut usize, name: *const c_char, class: u32) {
    // SAFETY: the caller's guarantee.
    unsafe {
        object(c).write(LockObject {
            name,
            flags: class << LO_CLASSSHIFT | LO_INITIALIZED,
            data: 0,
            witness: Box::into_raw(Box::new(Shared::new())).cast(),
        });
        c.write(NO_WRITER);
    }
}

/// # Safety
///
/// As `shared_init()`, initialised and free.
unsafe fn shared_destroy(c: *mut usize) {
    // SAFETY: the caller's guarantee.
    unsafe {
        drop(Box::from_raw((*object(c)).witness.cast::<Shared>()));
        (*object(c)).witness = null_mut();
    }
}

/// # Safety
///
/// As `shared()`.
unsafe fn write_lock(c: *mut usize) {
    // SAFETY: the caller's guarantee.
    unsafe {
        if word(c).load(Ordering::Relaxed) == me() {
            (*object(c)).data += 1;
            return;
        }
        shared(c).wait_until(Shared::try_write);
        word(c).store(me(), Ordering::Relaxed);
    }
}

/// # Safety
///
/// As `shared()`, write-held by this thread.
unsafe fn write_unlock(c: *mut usize) {
    // SAFETY: the caller's guarantee.
    unsafe {
        if (*object(c)).data > 0 {
            (*object(c)).data -= 1;
            return;
        }
        word(c).store(NO_WRITER, Ordering::Relaxed);
        shared(c).unlock_write();
    }
}

/// # Safety
///
/// As `shared()`.
unsafe fn try_write_lock(c: *mut usize) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe {
        if word(c).load(Ordering::Relaxed) == me() {
            (*object(c)).data += 1;
            return 1;
        }
        if shared(c).try_write() {
            word(c).store(me(), Ordering::Relaxed);
            return 1;
        }
        0
    }
}

/// # Safety
///
/// As `shared()`, read-held.
unsafe fn upgrade(c: *mut usize) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe {
        if shared(c).try_upgrade() {
            word(c).store(me(), Ordering::Relaxed);
            return 1;
        }
        0
    }
}

/// # Safety
///
/// As `shared()`, write-held once.
unsafe fn downgrade(c: *mut usize) {
    // SAFETY: the caller's guarantee.
    unsafe {
        word(c).store(NO_WRITER, Ordering::Relaxed);
        shared(c).downgrade();
    }
}

macro_rules! exported {
    ($($(#[$doc:meta])* $name:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)? $body:block)*) => {$(
        $(#[$doc])*
        ///
        /// # Safety
        ///
        /// `c` is such a lock's word, in the state FreeBSD requires.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
            // SAFETY: the caller's guarantee.
            unsafe { $body }
        }
    )*};
}

type File = *const c_char;

exported! {
    /// `rw_init_flags()`.
    _rw_init_flags(c: *mut usize, name: *const c_char, _opts: c_int) { shared_init(c, name, CLASS_RW) }
    /// `rw_destroy()`.
    _rw_destroy(c: *mut usize) { shared_destroy(c) }
    /// `rw_wowned()`.
    _rw_wowned(c: *mut usize) -> c_int { c_int::from(word(c).load(Ordering::Relaxed) == me()) }
    /// `rw_wlock()`.
    _rw_wlock_cookie(c: *mut usize, _f: File, _l: c_int) { write_lock(c) }
    /// `rw_wunlock()`.
    _rw_wunlock_cookie(c: *mut usize, _f: File, _l: c_int) { write_unlock(c) }
    /// `rw_try_wlock()`.
    __rw_try_wlock(c: *mut usize, _f: File, _l: c_int) -> c_int { try_write_lock(c) }
    /// `rw_rlock()`.
    __rw_rlock(c: *mut usize, _f: File, _l: c_int) { shared(c).wait_until(Shared::try_read) }
    /// `rw_try_rlock()`.
    __rw_try_rlock(c: *mut usize, _f: File, _l: c_int) -> c_int { c_int::from(shared(c).try_read()) }
    /// `rw_runlock()`.
    _rw_runlock_cookie(c: *mut usize, _f: File, _l: c_int) { shared(c).unlock_read() }
    /// `rw_try_upgrade()`.
    __rw_try_upgrade(c: *mut usize, _f: File, _l: c_int) -> c_int { upgrade(c) }
    /// `rw_downgrade()`.
    __rw_downgrade(c: *mut usize, _f: File, _l: c_int) { downgrade(c) }
    /// `sx_xlock()`; interruption is not modelled, as in `_sx_xlock_hard`.
    _sx_xlock(s: *mut u8, _o: c_int, _f: File, _l: c_int) -> c_int { write_lock(sx(s)); 0 }
    /// `sx_xunlock()`.
    _sx_xunlock(s: *mut u8, _f: File, _l: c_int) { write_unlock(sx(s)) }
    /// `sx_try_xlock()`.
    sx_try_xlock_(s: *mut u8, _f: File, _l: c_int) -> c_int { try_write_lock(sx(s)) }
    /// `sx_slock()`.
    _sx_slock(s: *mut u8, _o: c_int, _f: File, _l: c_int) -> c_int { shared(sx(s)).wait_until(Shared::try_read); 0 }
    /// `sx_sunlock()`.
    _sx_sunlock(s: *mut u8, _f: File, _l: c_int) { shared(sx(s)).unlock_read() }
    /// `sx_try_slock()`.
    sx_try_slock_(s: *mut u8, _f: File, _l: c_int) -> c_int { c_int::from(shared(sx(s)).try_read()) }
    /// `sx_try_upgrade()`.
    sx_try_upgrade_(s: *mut u8, _f: File, _l: c_int) -> c_int { upgrade(sx(s)) }
    /// `sx_downgrade()`.
    sx_downgrade_(s: *mut u8, _f: File, _l: c_int) { downgrade(sx(s)) }
    /// `sx_init_flags()`.
    sx_init_flags(s: *mut u8, name: *const c_char, _o: c_int) { shared_init(sx(s), name, CLASS_SX) }
    /// `sx_destroy()`.
    sx_destroy(s: *mut u8) { shared_destroy(sx(s)) }
    /// `rw_rlock()`, with `LOCK_DEBUG` 0: these take the lock, not its word.
    __rw_rlock_int(r: *mut u8) { shared(sx(r)).wait_until(Shared::try_read) }
    /// `rw_runlock()`, likewise.
    _rw_runlock_cookie_int(r: *mut u8) { shared(sx(r)).unlock_read() }
    /// `rw_try_upgrade()`, likewise.
    __rw_try_upgrade_int(r: *mut u8) -> c_int { upgrade(sx(r)) }
    /// `rw_downgrade()`, likewise.
    __rw_downgrade_int(r: *mut u8) { downgrade(sx(r)) }
    /// `rw_try_rlock()`, likewise.
    __rw_try_rlock_int(r: *mut u8) -> c_int { c_int::from(shared(sx(r)).try_read()) }
    /// `sx_slock()`, likewise.
    _sx_slock_int(s: *mut u8, _o: c_int) -> c_int { shared(sx(s)).wait_until(Shared::try_read); 0 }
    /// `sx_sunlock()`, likewise.
    _sx_sunlock_int(s: *mut u8) { shared(sx(s)).unlock_read() }
    /// `sx_try_slock()`, likewise.
    sx_try_slock_int(s: *mut u8) -> c_int { c_int::from(shared(sx(s)).try_read()) }
    /// `sx_try_xlock()`, likewise.
    sx_try_xlock_int(s: *mut u8) -> c_int { try_write_lock(sx(s)) }
    /// `sx_try_upgrade()`, likewise.
    sx_try_upgrade_int(s: *mut u8) -> c_int { upgrade(sx(s)) }
    /// `sx_downgrade()`, likewise.
    sx_downgrade_int(s: *mut u8) { downgrade(sx(s)) }
    /// `RW_SYSINIT()`.
    rw_sysinit(a: *const InitArgs) { _rw_init_flags((*a).lock.add(COOKIE).cast(), (*a).name, (*a).opts) }
    /// `SX_SYSINIT()`.
    sx_sysinit(a: *const InitArgs) { sx_init_flags((*a).lock, (*a).name, (*a).opts) }
}

/// The word of a `struct sx` or `struct rwlock`.
fn sx(s: *mut u8) -> *mut usize {
    s.wrapping_add(COOKIE).cast()
}

// The classes `msleep()` and `lock_classes[]` go through.

/// `struct lock_class`.
#[repr(C)]
pub struct LockClass {
    name: *const c_char,
    flags: c_uint,
    assert: unsafe extern "C" fn(*const LockObject, c_int),
    ddb_show: *const c_void,
    lock: unsafe extern "C" fn(*mut LockObject, usize),
    owner: unsafe extern "C" fn(*const LockObject, *mut *mut c_void) -> c_int,
    unlock: unsafe extern "C" fn(*mut LockObject) -> usize,
    trylock: unsafe extern "C" fn(*mut LockObject, usize) -> c_int,
}

// SAFETY: constant after build.
unsafe impl Sync for LockClass {}

fn cookie(o: *const LockObject) -> *mut usize {
    o.wrapping_add(1).cast::<usize>().cast_mut()
}

/// `LOCK_CLASS(lock)->lc_assert`: no assertions in this build.
unsafe extern "C" fn class_assert(_o: *const LockObject, _what: c_int) {}

unsafe extern "C" fn mtx_class_lock(o: *mut LockObject, _how: usize) {
    // SAFETY: a `struct mtx`'s object.
    unsafe { __mtx_lock_flags(cookie(o), 0, core::ptr::null(), 0) }
}

unsafe extern "C" fn mtx_class_unlock(o: *mut LockObject) -> usize {
    // SAFETY: a held `struct mtx`'s object.
    unsafe { __mtx_unlock_flags(cookie(o), 0, core::ptr::null(), 0) };
    0
}

unsafe extern "C" fn mtx_class_trylock(o: *mut LockObject, _how: usize) -> c_int {
    // SAFETY: a `struct mtx`'s object.
    unsafe { _mtx_trylock_flags_(cookie(o), 0, core::ptr::null(), 0) }
}

unsafe extern "C" fn spin_class_lock(o: *mut LockObject, _how: usize) {
    // SAFETY: a spin `struct mtx`'s object.
    unsafe { __mtx_lock_spin_flags(cookie(o), 0, core::ptr::null(), 0) }
}

unsafe extern "C" fn spin_class_unlock(o: *mut LockObject) -> usize {
    // SAFETY: a held spin `struct mtx`'s object.
    unsafe { __mtx_unlock_spin_flags(cookie(o), 0, core::ptr::null(), 0) };
    0
}

unsafe extern "C" fn spin_class_trylock(o: *mut LockObject, _how: usize) -> c_int {
    // SAFETY: a spin `struct mtx`'s object.
    unsafe { __mtx_trylock_spin_flags(cookie(o), 0, core::ptr::null(), 0) }
}

unsafe extern "C" fn word_owner(o: *const LockObject, owner: *mut *mut c_void) -> c_int {
    // SAFETY: a lock's object, and FreeBSD's out pointer.
    unsafe {
        let w = cookie(o).read();
        let held = w != 0 && w != NO_WRITER;
        if held {
            owner.write(w as *mut c_void);
        }
        c_int::from(held)
    }
}

/// `kern_rwlock.c`, `kern_sx.c`: `how` is 1 for a read lock.
unsafe extern "C" fn shared_class_lock(o: *mut LockObject, how: usize) {
    // SAFETY: an `rw` or `sx`'s object.
    unsafe {
        if how != 0 {
            shared(cookie(o)).wait_until(Shared::try_read);
        } else {
            write_lock(cookie(o));
        }
    }
}

unsafe extern "C" fn shared_class_trylock(o: *mut LockObject, how: usize) -> c_int {
    // SAFETY: an `rw` or `sx`'s object.
    unsafe {
        if how != 0 {
            c_int::from(shared(cookie(o)).try_read())
        } else {
            try_write_lock(cookie(o))
        }
    }
}

unsafe extern "C" fn shared_class_unlock(o: *mut LockObject) -> usize {
    // SAFETY: a held `rw` or `sx`'s object.
    unsafe {
        if cookie(o).read() == me() {
            write_unlock(cookie(o));
            0
        } else {
            shared(cookie(o)).unlock_read();
            1
        }
    }
}

macro_rules! class {
    ($sym:ident, $name:literal, $flags:expr, $lock:ident, $unlock:ident, $trylock:ident) => {
        #[unsafe(no_mangle)]
        #[doc = concat!("FreeBSD's `", stringify!($sym), "`.")]
        pub static $sym: LockClass = LockClass {
            name: $name.as_ptr(),
            flags: $flags,
            assert: class_assert,
            ddb_show: core::ptr::null(),
            lock: $lock,
            owner: word_owner,
            unlock: $unlock,
            trylock: $trylock,
        };
    };
}

class!(lock_class_mtx_sleep, c"sleep mutex", LC_SLEEPLOCK | LC_RECURSABLE, mtx_class_lock, mtx_class_unlock, mtx_class_trylock);
class!(lock_class_mtx_spin, c"spin mutex", LC_SPINLOCK | LC_RECURSABLE, spin_class_lock, spin_class_unlock, spin_class_trylock);
class!(lock_class_rw, c"rw", LC_SLEEPLOCK | LC_RECURSABLE | LC_UPGRADABLE, shared_class_lock, shared_class_unlock, shared_class_trylock);
class!(lock_class_sx, c"sx", LC_SLEEPLOCK | LC_SLEEPABLE | LC_RECURSABLE | LC_UPGRADABLE, shared_class_lock, shared_class_unlock, shared_class_trylock);

class!(lock_class_rm, c"rm", LC_SLEEPLOCK | LC_RECURSABLE, mtx_class_lock, mtx_class_unlock, mtx_class_trylock);
class!(lock_class_rm_sleepable, c"sleepable rm", LC_SLEEPLOCK | LC_SLEEPABLE | LC_RECURSABLE, mtx_class_lock, mtx_class_unlock, mtx_class_trylock);
class!(lock_class_lockmgr, c"lockmgr", LC_SLEEPLOCK | LC_SLEEPABLE, mtx_class_lock, mtx_class_unlock, mtx_class_trylock);

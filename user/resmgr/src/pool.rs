//! Thread pools, `thread_pool_create()`: threads that each loop on
//! block, then handle, with between `lo_water` and `hi_water` of them
//! blocked and waiting for work, and at most `maximum` in all.

use core::ffi::{c_int, c_uint, c_void};
use core::ptr::null_mut;

use crate::compat::Mutex;
use kercall::tls::get_errno;

use crate::abi::{ThreadPool, ThreadPoolAttr};
use crate::mem::{calloc, free};

/// `POOL_FLAG_EXIT_SELF`, `POOL_FLAG_USE_SELF`: what
/// `thread_pool_start()` does with the calling thread.
const POOL_FLAG_EXIT_SELF: c_uint = 0x1;
const POOL_FLAG_USE_SELF: c_uint = 0x2;
/// `POOL_FLAG_EXITING`: the pool is being destroyed.
const POOL_FLAG_EXITING: c_uint = 0x4;
/// `POOL_ERROR_BLOCK`, `POOL_ERROR_HANDLER`, `POOL_ERROR_CONTEXT`, and
/// the flags that ask for the first two.
const POOL_ERROR_BLOCK: c_uint = 0x1;
const POOL_ERROR_HANDLER: c_uint = 0x2;
const POOL_ERROR_CONTEXT: c_uint = 0x4;
const POOL_FLAG_CALL_BLOCK_ERRF: c_uint = 0x40;
const POOL_FLAG_CALL_HANDLE_ERRF: c_uint = 0x80;

/// The pool's lock and the count of threads still starting: `props`.
pub struct State {
    lock: Mutex,
    starting: c_uint,
}

unsafe extern "C" {
    fn pthread_create(
        t: *mut usize,
        attr: *const c_void,
        f: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
        arg: *mut c_void,
    ) -> c_int;
    fn pthread_detach(t: usize) -> c_int;
    fn pthread_self() -> usize;
    fn pthread_exit(v: *mut c_void) -> !;
    fn pthread_setname_np(t: usize, name: *const core::ffi::c_char) -> c_int;
    fn sched_yield() -> c_int;
}

/// The pool's state, from its `props`: a reference apart from the pool's,
/// so that the pool's counts change under the state's lock.
fn state<'a>(props: *mut c_void) -> &'a mut State {
    // SAFETY: props is the State thread_pool_create() allocated, which
    // lives as long as the pool.
    unsafe { &mut *props.cast::<State>() }
}

/// A pool with a copy of `attr`; no thread runs until
/// `thread_pool_start()`. Null with `errno` set on failure.
///
/// # Safety
///
/// `attr` is valid, with the functions a pool needs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_pool_create(
    attr: *const ThreadPoolAttr,
    flags: c_uint,
) -> *mut ThreadPool {
    let pool = calloc::<ThreadPool>();
    let st = calloc::<State>();
    if pool.is_null() || st.is_null() {
        // SAFETY: either is null or allocated.
        unsafe { free(pool.cast()) };
        // SAFETY: as above.
        unsafe { free(st.cast()) };
        kercall::tls::set_errno(crate::abi::ENOMEM);
        return null_mut();
    }
    // SAFETY: allocated for a State.
    unsafe {
        st.write(State {
            lock: Mutex::new(),
            starting: 0,
        });
    }
    // SAFETY: allocated; attr is the caller's.
    let p = unsafe { &mut *pool };
    // SAFETY: the caller's guarantee: a plain structure.
    p.pool_attr = unsafe { attr.read() };
    if p.pool_attr.error_func.is_none() {
        p.pool_attr.error_func = Some(thread_pool_default_error_func);
    }
    p.flags = flags & !(POOL_FLAG_EXITING | 0x8);
    p.props = st.cast();
    pool
}

/// The default error function: nothing.
#[unsafe(no_mangle)]
pub extern "C" fn thread_pool_default_error_func(_flags: c_uint, _err: c_int) {}

/// Starts the pool: `lo_water` threads. With `POOL_FLAG_USE_SELF`, the
/// caller becomes one of them and does not return; with
/// `POOL_FLAG_EXIT_SELF`, the caller exits.
///
/// # Safety
///
/// `pool` is a pool `thread_pool_create()` made.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_pool_start(pool: *mut c_void) -> c_int {
    let pool = pool.cast::<ThreadPool>();
    // SAFETY: the caller's guarantee.
    let p = unsafe { &mut *pool };
    let a = &mut p.pool_attr;
    a.hi_water = a.hi_water.min(a.maximum);
    a.lo_water = a.lo_water.min(a.hi_water);
    {
        let st = state(p.props);
        let _guard = st.lock.lock();
        p.created += 1;
        st.starting += 1;
    }
    if p.flags & POOL_FLAG_USE_SELF != 0 {
        // SAFETY: the pool, for the calling thread.
        unsafe { context_thread(pool.cast()) };
    }
    let mut t = 0;
    // SAFETY: the pool outlives its threads.
    if unsafe { pthread_create(&mut t, p.pool_attr.attr, context_thread, pool.cast()) } != 0 {
        let st = state(p.props);
        let _guard = st.lock.lock();
        p.created -= 1;
        st.starting -= 1;
        return -1;
    }
    if p.flags & POOL_FLAG_EXIT_SELF != 0 {
        // SAFETY: no preconditions.
        let me = unsafe { pthread_self() };
        // SAFETY: as above.
        unsafe { pthread_detach(me) };
        // SAFETY: as above.
        unsafe { pthread_exit(null_mut()) };
    }
    0
}

/// A pool thread's start: its context, then the loop.
unsafe extern "C" fn context_thread(arg: *mut c_void) -> *mut c_void {
    let pool = arg.cast::<ThreadPool>();
    // SAFETY: the pool outlives its threads.
    let p = unsafe { &mut *pool };
    // SAFETY: no preconditions.
    let me = unsafe { pthread_self() };
    // SAFETY: as above.
    unsafe { pthread_detach(me) };
    let alloc = p.pool_attr.context_alloc;
    // SAFETY: the pool's allocator, with its handle.
    let ctp = alloc.map_or(null_mut(), |f| unsafe { f(p.pool_attr.handle) });
    if ctp.is_null() {
        {
            let st = state(p.props);
            let _guard = st.lock.lock();
            p.created -= 1;
            st.starting -= 1;
            p.flags |= 0x20;
        }
        if let Some(f) = p.pool_attr.error_func {
            // SAFETY: the pool's error function.
            unsafe { f(POOL_ERROR_CONTEXT, 0) };
        }
        // SAFETY: no preconditions.
        unsafe { pthread_exit(null_mut()) };
    }
    if !p.pool_attr.tid_name.is_null() {
        // SAFETY: a string the attributes hold.
        let me = unsafe { pthread_self() };
        // SAFETY: as above.
        unsafe { pthread_setname_np(me, p.pool_attr.tid_name) };
    }
    update(p, 1, true);
    // SAFETY: as above.
    unsafe { pool_loop(p, ctp) }
}

/// Blocks, handles, and repeats, while the pool wants this thread.
///
/// # Safety
///
/// `ctp` is a context the pool's allocator made.
unsafe fn pool_loop(p: &mut ThreadPool, mut ctp: *mut c_void) -> ! {
    let a = &p.pool_attr;
    let (block, handler, free_ctx) = (a.block_func, a.handler_func, a.context_free);
    loop {
        // SAFETY: the pool's block function, with this thread's context.
        let got = block.map_or(ctp, |f| unsafe { f(ctp) });
        if got.is_null() {
            if p.flags & POOL_FLAG_CALL_BLOCK_ERRF != 0
                && let Some(f) = p.pool_attr.error_func
            {
                // SAFETY: the pool's error function.
                unsafe { f(POOL_ERROR_BLOCK, get_errno()) };
            }
        } else {
            ctp = got;
        }
        update(p, -1, false);
        if !got.is_null() {
            // SAFETY: the pool's handler, with the received context.
            let r = handler.map_or(0, |f| unsafe { f(got) });
            if r == -1
                && p.flags & POOL_FLAG_CALL_HANDLE_ERRF != 0
                && let Some(f) = p.pool_attr.error_func
            {
                // SAFETY: as above.
                unsafe { f(POOL_ERROR_HANDLER, get_errno()) };
            }
        }
        let st = state(p.props);
        let guard = st.lock.lock();
        let exiting = p.flags & POOL_FLAG_EXITING != 0;
        if exiting || p.waiting >= c_uint::from(p.pool_attr.hi_water) {
            p.created -= 1;
            drop(guard);
            if let Some(f) = free_ctx {
                // SAFETY: the pool's freer, with this thread's context.
                unsafe { f(ctp) };
            }
            // SAFETY: no preconditions.
            unsafe { pthread_exit(null_mut()) };
        }
        p.waiting += 1;
    }
}

/// Counts a thread in or out of the waiting ones (`delta`), and one that
/// finished starting; starts threads if fewer than `lo_water` wait.
fn update(p: &mut ThreadPool, delta: i32, started: bool) {
    let st = state(p.props);
    let guard = st.lock.lock();
    p.waiting = p.waiting.wrapping_add_signed(delta);
    if started {
        st.starting -= 1;
    }
    let a = &p.pool_attr;
    let ready = p.waiting + st.starting;
    let lo = c_uint::from(a.lo_water);
    if p.flags & POOL_FLAG_EXITING != 0 || ready >= lo || p.created >= c_uint::from(a.maximum) {
        return;
    }
    let want = if p.created < lo {
        lo - p.created
    } else {
        c_uint::from(a.increment).max(1)
    };
    let want = want
        .min(c_uint::from(a.hi_water).saturating_sub(ready).max(1))
        .min(c_uint::from(a.maximum) - p.created);
    p.created += want;
    st.starting += want;
    drop(guard);
    let pool = core::ptr::from_mut(p).cast::<c_void>();
    for _ in 0..want {
        let mut t = 0;
        // SAFETY: the pool outlives its threads.
        if unsafe { pthread_create(&mut t, p.pool_attr.attr, context_thread, pool) } != 0 {
            let st = state(p.props);
            let _guard = st.lock.lock();
            p.created -= 1;
            st.starting -= 1;
            p.flags |= 0x20;
        }
    }
}

/// Stops the pool: wakes every thread, waits for them to exit, frees it.
///
/// # Safety
///
/// `pool` is a pool; the caller is not one of its threads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_pool_destroy(pool: *mut ThreadPool) -> c_int {
    // SAFETY: the caller's guarantee.
    let p = unsafe { &mut *pool };
    {
        let st = state(p.props);
        let _guard = st.lock.lock();
        p.pool_attr.lo_water = 0;
        p.pool_attr.hi_water = 0;
        p.pool_attr.maximum = 0;
        p.flags |= POOL_FLAG_EXITING;
    }
    let a = &p.pool_attr;
    // SAFETY: the pool's allocator.
    let ctp = a
        .context_alloc
        .map_or(null_mut(), |f| unsafe { f(a.handle) });
    loop {
        let created = {
            let st = state(p.props);
            let _guard = st.lock.lock();
            p.created
        };
        if created == 0 {
            break;
        }
        if !ctp.is_null()
            && let Some(f) = a.unblock_func
        {
            // SAFETY: the pool's unblock function.
            unsafe { f(ctp) };
        }
        // SAFETY: no preconditions.
        unsafe { sched_yield() };
    }
    if !ctp.is_null()
        && let Some(f) = a.context_free
    {
        // SAFETY: the pool's freer.
        unsafe { f(ctp) };
    }
    // SAFETY: allocated by thread_pool_create().
    unsafe { free(p.props) };
    // SAFETY: as above.
    unsafe { free(pool.cast()) };
    0
}

/// Changes the pool's limits: `thread_pool_control()`.
///
/// # Safety
///
/// `pool` is a pool; `attr` is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_pool_control(
    pool: *mut ThreadPool,
    attr: *const ThreadPoolAttr,
    _lower: u16,
    _upper: u16,
    flags: c_uint,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let p = unsafe { &mut *pool };
    // SAFETY: as above.
    let a = unsafe { &*attr };
    let st = state(p.props);
    let _guard = st.lock.lock();
    if flags & 0x1 != 0 {
        p.pool_attr.hi_water = a.hi_water;
    }
    if flags & 0x2 != 0 {
        p.pool_attr.lo_water = a.lo_water;
    }
    if flags & 0x4 != 0 {
        p.pool_attr.maximum = a.maximum;
    }
    if flags & 0x8 != 0 {
        p.pool_attr.increment = a.increment;
    }
    0
}

/// Changes the pool's limits; negative values keep the current ones.
///
/// # Safety
///
/// `pool` is a pool.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn thread_pool_limits(
    pool: *mut ThreadPool,
    lowater: c_int,
    hiwater: c_int,
    maximum: c_int,
    increment: c_int,
    _flags: c_uint,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let p = unsafe { &mut *pool };
    let st = state(p.props);
    let _guard = st.lock.lock();
    let a = &mut p.pool_attr;
    for (v, f) in [
        (lowater, &mut a.lo_water),
        (hiwater, &mut a.hi_water),
        (maximum, &mut a.maximum),
        (increment, &mut a.increment),
    ] {
        if v >= 0 {
            *f = v as u16;
        }
    }
    0
}

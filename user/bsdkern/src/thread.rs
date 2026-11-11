//! The pthreads under `kern/thread.c`, and its allocations, which the
//! kernel's `malloc()` can't serve: it isn't up yet, and needs threads.

use core::ffi::{c_char, c_int, c_void};
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ffi::CStr;

const HEADER: usize = 16;
const EAGAIN: c_int = 35;

unsafe extern "C" {
    fn bsdkern_thread_start(arg: *mut c_void);
}

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size + HEADER, HEADER).expect("bsdkern: allocation size")
}

/// Zeroed memory, from libc's allocator.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_zalloc(size: usize) -> *mut c_void {
    // SAFETY: the layout's size is not zero; the size goes in the header.
    unsafe {
        let p = alloc_zeroed(layout(size));
        assert!(!p.is_null(), "bsdkern: out of memory");
        p.cast::<usize>().write(size);
        p.add(HEADER).cast()
    }
}

/// Frees what `bsdkern_zalloc()` gave.
///
/// # Safety
///
/// `p` came from `bsdkern_zalloc()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_free(p: *mut c_void) {
    // SAFETY: the caller's guarantee; the header holds the size.
    unsafe {
        let p = p.cast::<u8>().sub(HEADER);
        dealloc(p, layout(p.cast::<usize>().read()));
    }
}

struct Start(*mut c_void);
// SAFETY: handed once to the new thread.
unsafe impl Send for Start {}

/// A detached thread running `bsdkern_thread_start(start)`.
///
/// # Safety
///
/// `start` is for `bsdkern_thread_start()`, `name` a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_thread_spawn(start: *mut c_void, name: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    let name = unsafe { CStr::from_ptr(name) }.to_string_lossy().into_owned();
    let start = Start(start);
    let thread = std::thread::Builder::new().name(name).spawn(move || {
        let start = start;
        // SAFETY: the caller's guarantee.
        unsafe { bsdkern_thread_start(start.0) };
    });
    if thread.is_ok() { 0 } else { EAGAIN }
}

/// `kthread_exit()`: ends this thread.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_thread_exit() -> ! {
    unsafe extern "C" {
        fn pthread_exit(value: *mut c_void) -> !;
    }
    // SAFETY: called on a thread of ours, in FreeBSD's C frames.
    unsafe { pthread_exit(core::ptr::null_mut()) }
}

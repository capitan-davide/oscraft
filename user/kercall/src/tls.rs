//! The thread's `errno` and the process ID, from FreeBSD's libc (where
//! oscraft-libc read its own thread-local storage).

use core::ffi::c_int;

unsafe extern "C" {
    fn __error() -> *mut c_int;
    fn getpid() -> c_int;
}

/// Sets the calling thread's `errno`.
pub fn set_errno(error: c_int) {
    // SAFETY: __error() returns the calling thread's errno.
    unsafe { *__error() = error };
}

/// The calling thread's `errno`.
pub fn get_errno() -> c_int {
    // SAFETY: __error() returns the calling thread's errno.
    unsafe { *__error() }
}

/// The calling process's ID.
pub fn pid() -> c_int {
    // SAFETY: a call without preconditions.
    unsafe { getpid() }
}

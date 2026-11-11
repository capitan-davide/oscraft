//! `errno`: where a failed call's error number goes, and FreeBSD's
//! numbering of it.
//!
//! FreeBSD's `libsys` keeps the error number of an unthreaded program in
//! its own variable and lets the threading library select a per-thread
//! one (`__set_error_selector()`); `__error()` returns the current
//! choice and `errno` is `*__error()`. That is FreeBSD's `__error.c`,
//! compiled into libsys next to this crate; the stubs here only call it.

use core::ffi::c_int;

unsafe extern "C" {
    /// `__error()`: the calling thread's `errno`; FreeBSD's `__error.c`,
    /// built into libsys with this crate.
    pub fn __error() -> *mut c_int;
    /// `write(2)`'s stub, for a stub's report.
    fn __sys_write(fd: c_int, buf: *const core::ffi::c_void, n: usize) -> isize;
}

/// What `cerror` does: stores the error in `errno` and returns -1. The
/// kernel gives a FreeBSD process FreeBSD's numbers, so there is nothing
/// to translate.
pub fn fail(error: c_int) -> isize {
    // SAFETY: __error() returns the thread's errno.
    unsafe { *__error() = error };
    -1
}

/// FreeBSD's number for QNX's error `e` in a QNX server's payload, which
/// the kernel doesn't see (`SO_ERROR`); a QNX error FreeBSD has no name
/// for is EIO.
pub fn from_qnx(e: c_int) -> c_int {
    qnxabi::errno::from_qnx(e).unwrap_or(proto::errno::EIO)
}

/// A system call this crate does not implement: says so on the standard
/// error, once per call site, and fails with ENOSYS.
pub fn not_implemented(name: &str) -> isize {
    let mut line = [0u8; 96];
    let head = b"libsys: not implemented: ";
    line[..head.len()].copy_from_slice(head);
    let n = name.len().min(line.len() - head.len() - 1);
    line[head.len()..head.len() + n].copy_from_slice(&name.as_bytes()[..n]);
    line[head.len() + n] = b'\n';
    // SAFETY: the line is on this stack, for its length.
    unsafe { __sys_write(2, line.as_ptr().cast(), head.len() + n + 1) };
    fail(proto::errno::ENOSYS)
}

/// Reports `what` and a number on the standard error: a diagnostic.
pub fn report(what: &str, n: i64) {
    let mut line = [0u8; 96];
    let head = b"libsys: ";
    let mut i = 0;
    for b in head.iter().chain(what.as_bytes()).chain(b": ") {
        if i < line.len() - 24 {
            line[i] = *b;
            i += 1;
        }
    }
    let mut digits = [0u8; 20];
    let mut d = 0;
    let (mut v, neg) = (n.unsigned_abs(), n < 0);
    loop {
        digits[d] = b'0' + (v % 10) as u8;
        d += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    if neg {
        line[i] = b'-';
        i += 1;
    }
    while d > 0 {
        d -= 1;
        line[i] = digits[d];
        i += 1;
    }
    line[i] = b'\n';
    // SAFETY: the line is on this stack, for its length.
    unsafe { __sys_write(2, line.as_ptr().cast(), i + 1) };
}

/// The thread's errno (FreeBSD's number), to keep across a call.
pub fn last() -> c_int {
    // SAFETY: __error() returns the thread's errno.
    unsafe { *__error() }
}

/// Sets the thread's errno (FreeBSD's number).
pub fn set_last(e: c_int) {
    // SAFETY: __error() returns the thread's errno.
    unsafe { *__error() = e };
}

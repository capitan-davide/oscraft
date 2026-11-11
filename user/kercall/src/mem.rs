//! Memory for the library's own use: FreeBSD's `mmap()` and `munmap()`
//! (oscraft-libc had the memory manager's messages here).

use core::ffi::{c_int, c_void};

use crate::tls;

unsafe extern "C" {
    fn mmap(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> *mut c_void;
    fn munmap(addr: *mut c_void, len: usize) -> c_int;
}

/// A mapping of `len` bytes: the address, or the error.
///
/// # Safety
///
/// As `mmap()`.
pub unsafe fn map(addr: *mut c_void, len: usize, prot: c_int, flags: c_int, fd: c_int, offset: i64) -> Result<*mut c_void, c_int> {
    // SAFETY: the caller's guarantee.
    let p = unsafe { mmap(addr, len, prot, flags, fd, offset) };
    if p as isize == -1 { Err(tls::get_errno()) } else { Ok(p) }
}

/// Unmaps `len` bytes at `addr`.
///
/// # Safety
///
/// As `munmap()`.
pub unsafe fn unmap(addr: *mut c_void, len: usize, _flags: c_int) -> Result<(), c_int> {
    // SAFETY: the caller's guarantee.
    if unsafe { munmap(addr, len) } == 0 { Ok(()) } else { Err(tls::get_errno()) }
}

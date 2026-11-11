//! Memory, from the ISO C layer's allocator: the framework's lists and
//! tables are C-style, of allocated nodes.

use core::ffi::c_void;

unsafe extern "C" {
    #[link_name = "calloc"]
    fn c_calloc(n: usize, size: usize) -> *mut c_void;
    #[link_name = "free"]
    fn c_free(p: *mut c_void);
}

/// `size` zeroed bytes, or null.
pub fn calloc_bytes(size: usize) -> *mut u8 {
    // SAFETY: the C library's allocator, with any size.
    unsafe { c_calloc(1, size) }.cast()
}

/// Zeroed room for one `T`, or null. Every `T` the framework allocates
/// is valid when zeroed, or written before use.
pub fn calloc<T>() -> *mut T {
    calloc_bytes(size_of::<T>()).cast()
}

/// Frees what [`calloc`] returned.
///
/// # Safety
///
/// `p` is null or an allocation not yet freed.
pub unsafe fn free(p: *mut c_void) {
    // SAFETY: the caller's guarantee.
    unsafe { c_free(p) };
}

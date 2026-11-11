//! What the C part needs of the program it runs in.

use core::ffi::{CStr, c_char, c_int};
use std::io::Write;
use std::time::Duration;

unsafe extern "C" {
    fn bsdkern_kernel_start(cpus: c_int);
}

unsafe extern "C" {
    static mut bootverbose: c_int;
}

/// FreeBSD's verbose boot: each SYSINIT is printed.
pub fn verbose() {
    // SAFETY: before anything starts.
    unsafe { bootverbose = 1 };
}

/// Adds `name=value` to FreeBSD's kernel environment, as the loader's
/// (device hints, tunables); before `start()`. False if it doesn't fit.
pub fn kenv(s: &str) -> bool {
    unsafe extern "C" {
        fn bsdkern_kenv_add(s: *const c_char) -> c_int;
    }
    let c = std::ffi::CString::new(s).unwrap_or_default();
    // SAFETY: a C string, copied; before anything starts.
    unsafe { bsdkern_kenv_add(c.as_ptr()) == 0 }
}

/// A variable of FreeBSD's kernel environment (`-e`, `kenv`), if set.
pub fn kenv_get(name: &str) -> Option<String> {
    unsafe extern "C" {
        fn bsdkern_kenv_get(name: *const c_char, buf: *mut c_char, len: c_int) -> c_int;
    }
    let c = std::ffi::CString::new(name).ok()?;
    let mut buf = [0u8; 128];
    // SAFETY: a C string and a buffer of its length.
    if unsafe { bsdkern_kenv_get(c.as_ptr(), buf.as_mut_ptr().cast(), buf.len() as c_int) } == 0 {
        return None;
    }
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    Some(String::from_utf8_lossy(&buf[..n]).into_owned())
}

/// Loads the module at `path` with FreeBSD's `kldload(2)` (its linker over
/// `dlopen()`, `kern/link_dl.c`), after `start()`: its drivers then probe
/// the devices already found. The file's ID, or an errno.
pub fn kldload(path: &str) -> Result<i64, c_int> {
    unsafe extern "C" {
        fn bsdkern_kld(number: c_int, a0: c_int, a1: c_int, buf: *mut c_char, ret: *mut i64) -> c_int;
    }
    /// `SYS_kldload` (`<sys/syscall.h>`).
    const SYS_KLDLOAD: c_int = 304;
    let mut buf = path.as_bytes().to_vec();
    buf.push(0);
    let mut ret = 0i64;
    // SAFETY: a NUL-terminated path in a buffer of ours.
    let e = unsafe { bsdkern_kld(SYS_KLDLOAD, 0, 0, buf.as_mut_ptr().cast(), &raw mut ret) };
    if e != 0 { Err(e) } else { Ok(ret) }
}

/// Whether the server was started verbose (`-v`).
pub fn is_verbose() -> bool {
    // SAFETY: set once before anything starts.
    unsafe { bootverbose != 0 }
}

/// Starts FreeBSD's code on `cpus` slots.
pub fn start(cpus: usize) {
    crate::lock::init();
    // SAFETY: once, from the first thread.
    unsafe { bsdkern_kernel_start(cpus as c_int) };
}

/// Sleeps for `ns` nanoseconds.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_nanosleep(ns: u64) {
    std::thread::sleep(Duration::from_nanos(ns));
}

/// `printf()` of FreeBSD's code.
///
/// # Safety
///
/// `s` holds `n` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_console(s: *const c_char, n: usize) {
    // A server's console isn't the terminal's: said only when asked (`-v`).
    // SAFETY: only read.
    if unsafe { bootverbose } == 0 {
        return;
    }
    // SAFETY: the caller's guarantee.
    let bytes = unsafe { core::slice::from_raw_parts(s.cast::<u8>(), n) };
    let _ = std::io::stderr().write_all(bytes);
}

/// `panic()` of FreeBSD's code.
///
/// # Safety
///
/// `s` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_panic(s: *const c_char) -> ! {
    // SAFETY: the caller's guarantee.
    eprintln!("{}: panic: {}", me(), unsafe { CStr::from_ptr(s) }.to_string_lossy());
    std::process::abort();
}

/// A call to what the server must never reach: `kern/stubs.c`.
///
/// # Safety
///
/// `s` is a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_stub(s: *const c_char) -> ! {
    // SAFETY: the caller's guarantee.
    eprintln!("{}: not implemented: {}", me(), unsafe { CStr::from_ptr(s) }.to_string_lossy());
    std::process::abort();
}


/// Stops the server if a FreeBSD call failed.
pub fn check(what: &str, e: c_int) {
    assert_eq!(e, 0, "{}: {what}", me());
}

/// The server's name, for its messages: its program's.
pub fn me() -> String {
    unsafe extern "C" {
        fn getprogname() -> *const c_char;
    }
    // SAFETY: libc's, a C string for the life of the process.
    unsafe { CStr::from_ptr(getprogname()) }.to_string_lossy().into_owned()
}

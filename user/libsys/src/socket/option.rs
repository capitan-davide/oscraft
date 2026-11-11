//! The options of a socket: `_IO_MSG`s of `io-sock`.

use core::ffi::{c_int, c_void};

use proto::errno::EINVAL;
use proto::io::Msg;

use super::{MSG_GETOPT, MSG_SETOPT, message, request};
use crate::errno::fail;
use crate::kercall::Iov;

/// `io_sock_gopt_t` and `io_sock_sopt_t`.
#[repr(C)]
struct Opt {
    msg: Msg,
    level: u32,
    name: u32,
    len: u32,
    zero: u32,
}

const OPT_SIZE: u16 = size_of::<Opt>() as u16;

/// `getsockopt(s, level, name, val, len)`.
///
/// # Safety
///
/// `val` is writable for `*len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getsockopt(s: c_int, level: c_int, name: c_int, val: *mut c_void, len: *mut u32) -> c_int {
    if val.is_null() || len.is_null() {
        return fail(EINVAL) as c_int;
    }
    let mut opt = Opt {
        msg: message(OPT_SIZE, MSG_GETOPT),
        level: level as u32,
        name: name as u32,
        // SAFETY: the caller's guarantee.
        len: unsafe { *len },
        zero: 0,
    };
    let cap = opt.len as usize;
    let parts = [Iov::raw((&raw mut opt).cast(), size_of::<Opt>()), Iov::raw(val, cap)];
    // SAFETY: the request, and the caller's buffer: the reply goes over
    // both.
    match unsafe { request(s, &parts, &parts) } {
        Ok(_) => {
            // SAFETY: the caller's guarantee.
            unsafe { *len = opt.len };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `setsockopt(s, level, name, val, len)`.
///
/// # Safety
///
/// `val` is readable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_setsockopt(s: c_int, level: c_int, name: c_int, val: *const c_void, len: u32) -> c_int {
    if val.is_null() {
        return fail(EINVAL) as c_int;
    }
    let opt = Opt {
        msg: message(OPT_SIZE.wrapping_add(len as u16), MSG_SETOPT),
        level: level as u32,
        name: name as u32,
        len,
        zero: 0,
    };
    let parts = [Iov::raw((&raw const opt).cast(), size_of::<Opt>()), Iov::raw(val, len as usize)];
    // SAFETY: the request and the caller's value.
    match unsafe { request(s, &parts, &[]) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

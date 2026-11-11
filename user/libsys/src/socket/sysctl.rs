//! `sysctl()` through `io-sock`: the stack's variables (`net.*`, the
//! routing tables `getifaddrs()` lists, `kern.hostname`, the name lookup
//! `sysctl.name2oid`) are read and set with an `_IO_MSG` on a socket
//! made for the call.

use core::ffi::{c_int, c_void};

use proto::io::Msg;

use super::{MSG_SYSCTL, message, request};
use crate::kercall::Iov;

const AF_INET: c_int = 2;
const SOCK_DGRAM: c_int = 2;
const SOCK_CLOEXEC: c_int = 0x1000_0000;

/// `io_sock_sysctl_t`.
#[repr(C)]
struct Sysctl {
    msg: Msg,
    namelen: u32,
    oldlen: usize,
    newlen: usize,
}

/// What a request found: the value's length, and whether it was cut.
pub struct Found {
    pub len: usize,
    pub cut: bool,
}

/// `io_sock_sysctl_t` on connection `fd`.
///
/// # Safety
///
/// `name` holds `namelen` ints; the buffers are valid for their lengths.
pub unsafe fn ask(fd: c_int, name: &[c_int], oldp: *mut c_void, oldlen: usize, newp: *const c_void, newlen: usize) -> Result<Found, c_int> {
    let names = size_of_val(name);
    let mut sys = Sysctl {
        msg: message((size_of::<Sysctl>() + names) as u16, MSG_SYSCTL),
        namelen: name.len() as u32,
        oldlen: if oldp.is_null() { 0 } else { oldlen },
        newlen,
    };
    let header = Iov::raw((&raw mut sys).cast(), size_of::<Sysctl>());
    let out = [header, Iov::raw(oldp.cast_const(), sys.oldlen)];
    let parts = [header, Iov::raw(name.as_ptr().cast(), names), Iov::raw(newp, newlen)];
    let n = if oldp.is_null() { 1 } else { 2 };
    // SAFETY: the request, the name, and the caller's buffers.
    let status = unsafe { request(fd, &parts, &out[..n]) }?;
    Ok(Found { len: sys.oldlen, cut: status == 1 })
}

/// Reads and sets variable `name` in the network stack.
///
/// # Safety
///
/// As `ask()`.
pub unsafe fn via_socket(name: &[c_int], oldp: *mut c_void, oldlen: usize, newp: *const c_void, newlen: usize) -> Result<usize, c_int> {
    let fd = super::call::socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0)?;
    // SAFETY: the caller's guarantee.
    let r = unsafe { ask(fd, name, oldp, oldlen, newp, newlen) };
    let _ = crate::fs::__sys_close(fd);
    r.map(|f| f.len)
}

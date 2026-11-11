//! Sending: `sendto()` and `sendmsg()`, writes with the `_IO_XTYPE_TCPIP`
//! extensions that carry the name, the flags and the control data.

use core::ffi::{c_int, c_void};
use core::ptr::null;

use proto::errno::EMSGSIZE;
use proto::io::{IO_WRITE, IO_WRITE64, ReadWrite};

use super::address::{Use, fs_name};
use super::call::wire_name;
use super::control::{Wire, release, to_wire};
use super::{Msghdr, Parts, request};
use crate::fd::server_info;
use crate::errno::fail;
use crate::kercall::Iov;

/// `_IO_XTYPE_TCPIP`, `_IO_XTYPE_TCPIP_MSG`.
pub const XTYPE_TCPIP: u32 = 3;
const XTYPE_TCPIP_MSG: u32 = 4;

/// `io_sock_sendto_t`.
#[repr(C)]
struct Sendto {
    write: ReadWrite,
    flags: c_int,
    addrlen: u32,
}

/// `io_sock_sendmsg_t`.
#[repr(C)]
struct Sendmsg {
    write: ReadWrite,
    flags: c_int,
    addrlen: u16,
    controllen: u16,
}

/// `sendto(fd, buf, len, flags, to, tolen)`.
///
/// # Safety
///
/// `buf` is readable for `len` bytes, `to` for `tolen` unless null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sendto(fd: c_int, buf: *const c_void, len: usize, flags: c_int, to: *const c_void, tolen: u32) -> isize {
    // SAFETY: the caller's guarantee.
    if let Some(r) = unsafe { crate::logsock::send(fd, buf, len) } {
        return r;
    }
    let mut fs = None;
    if !to.is_null() {
        // SAFETY: the caller's guarantee.
        match unsafe { fs_name(0, fd, to, tolen, Use::Connect) } {
            Ok(found) => fs = found,
            Err(e) => return fail(e),
        }
    }
    let (name, namelen) = if to.is_null() { (null(), 0) } else { wire_name(&fs, to, tolen) };
    let msg = Sendto {
        write: ReadWrite {
            r#type: if len >> 32 == 0 { IO_WRITE } else { IO_WRITE64 },
            combine_len: size_of::<Sendto>() as u16,
            nbytes: len as u32,
            xtype: XTYPE_TCPIP,
            nbytes_hi: (len >> 32) as u32,
        },
        flags,
        addrlen: namelen,
    };
    let parts = [
        Iov::raw((&raw const msg).cast(), size_of::<Sendto>()),
        Iov::raw(name, namelen as usize),
        Iov::raw(buf, len),
    ];
    // SAFETY: the request, the name, and the caller's data.
    match unsafe { request(fd, &parts, &[]) } {
        Ok(n) => n as isize,
        Err(e) => fail(e),
    }
}

/// `sendmsg(fd, msg, flags)`.
///
/// # Safety
///
/// `m` is a valid message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sendmsg(fd: c_int, m: *const Msghdr, flags: c_int) -> isize {
    // SAFETY: the caller's guarantee.
    let src = unsafe { &*m };
    if src.iovlen < 1 {
        return fail(EMSGSIZE);
    }
    let pid = match server_info(fd) {
        Ok(i) => i.pid,
        Err(e) => return fail(e),
    };
    let mut fs = None;
    if !src.name.is_null() && src.namelen != 0 {
        // SAFETY: the caller's guarantee.
        match unsafe { fs_name(pid, fd, src.name, src.namelen, Use::Connect) } {
            Ok(found) => fs = found,
            Err(e) => return fail(e),
        }
    }
    let (name, namelen) = if src.name.is_null() { (null(), 0) } else { wire_name(&fs, src.name, src.namelen) };
    // SAFETY: the caller's guarantee.
    let control = match unsafe { to_wire(pid, src.control, if src.control.is_null() { 0 } else { src.controllen }) } {
        Ok(w) => w,
        Err(e) => return fail(e),
    };
    let r = send_parts(fd, src, flags, name, namelen, &control);
    // SAFETY: to_wire() made it for this control data.
    unsafe { release(&control, src.control) };
    r
}

/// The request of a message whose name and control data are ready.
fn send_parts(fd: c_int, src: &Msghdr, flags: c_int, name: *const c_void, namelen: u32, control: &Wire) -> isize {
    let controllen = control.len;
    let n = src.iovlen as usize;
    let mut parts = match Parts::new(n + 3) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let iov = parts.as_mut_ptr();
    let mut total = 0usize;
    // SAFETY: room for n + 3 parts; the message's parts are the caller's.
    unsafe {
        iov.add(1).write(Iov::raw(name, namelen as usize));
        iov.add(2).write(Iov::raw(control.ptr.cast_const(), controllen as usize));
        for i in 0..n {
            let part = src.iov.add(i).read();
            iov.add(3 + i).write(part);
            total += part.len;
        }
    }
    let head = Sendmsg {
        write: ReadWrite {
            r#type: IO_WRITE,
            combine_len: 0,
            nbytes: total as u32,
            xtype: XTYPE_TCPIP_MSG,
            nbytes_hi: 0,
        },
        flags,
        addrlen: namelen as u16,
        controllen: controllen as u16,
    };
    // SAFETY: part 0, in the room.
    unsafe { iov.write(Iov::raw((&raw const head).cast(), size_of::<Sendmsg>())) };
    // SAFETY: n + 3 parts were written.
    let list = unsafe { parts.slice(n + 3) };
    // SAFETY: each part is the message's.
    match unsafe { request(fd, list, &[]) } {
        Ok(r) => r as isize,
        Err(e) => fail(e),
    }
}

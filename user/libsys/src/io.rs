//! The file-descriptor calls. In QNX a file descriptor is a connection to
//! the resource manager that owns the file, and each call is a message
//! on it (`<sys/iomsg.h>`); the server's reply status is the result.

use core::ffi::{c_int, c_void};

use proto::io::{self as m, ReadWrite};

use crate::errno::fail;
use crate::kercall::{Iov, msg_sendv};

/// `write(fd, buf, nbytes)`: an `_IO_WRITE` message, header and data.
///
/// # Safety
///
/// `buf` is valid for reads of `nbytes` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_write(fd: c_int, buf: *const c_void, nbytes: usize) -> isize {
    let nbytes = nbytes.min(u32::MAX as usize);
    let header = ReadWrite {
        r#type: m::IO_WRITE,
        combine_len: size_of::<ReadWrite>() as u16,
        nbytes: nbytes as u32,
        xtype: m::IO_XTYPE_NONE,
        nbytes_hi: 0,
    };
    let message = [Iov::of(&header), Iov::raw(buf, nbytes)];
    // SAFETY: the header is on this stack, the data by the caller's word.
    match unsafe { msg_sendv(fd, &message, &[]) } {
        Ok(n) => n as isize,
        Err(e) => fail(e),
    }
}

/// `read(fd, buf, nbytes)`: an `_IO_READ` message, the data replied
/// straight into `buf`.
///
/// # Safety
///
/// `buf` is valid for writes of `nbytes` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_read(fd: c_int, buf: *mut c_void, nbytes: usize) -> isize {
    let nbytes = nbytes.min(u32::MAX as usize);
    let header = ReadWrite {
        r#type: m::IO_READ,
        combine_len: size_of::<ReadWrite>() as u16,
        nbytes: nbytes as u32,
        xtype: m::IO_XTYPE_NONE,
        nbytes_hi: 0,
    };
    let message = [Iov::of(&header)];
    let reply = [Iov::raw(buf, nbytes)];
    // SAFETY: as for write; the reply part is the caller's buffer.
    match unsafe { msg_sendv(fd, &message, &reply) } {
        Ok(n) => n as isize,
        Err(e) => fail(e),
    }
}

//! Sockets: FreeBSD's calls as the messages to `io-sock`, QNX's network
//! manager (itself FreeBSD's stack), each of them is. A socket is a
//! connection to the manager's node of its domain, `/dev/socket/<n>`;
//! the data calls are reads and writes with an `_IO_XTYPE_TCPIP*`
//! extension, the rest device commands and `_IO_MSG`s of manager
//! `_IOMGR_NETWORK`. The layouts (`sockaddr`, `msghdr`, `cmsghdr`, the
//! options, the interface requests) are FreeBSD's on both sides.
//!
//! `AF_UNIX` names in the file system and `SCM_RIGHTS` pass descriptors
//! as `dupfd` handles (`address`, `control`).

pub mod address;
pub mod call;
pub mod control;
pub mod netioctl;
pub mod option;
pub mod receive;
pub mod sysctl;
pub mod transmit;

use core::ffi::{c_int, c_void};

use proto::errno::{ENOSYS, ENOTSOCK};
use proto::io::{IO_MSG, Msg};

use crate::kercall::{Iov, msg_sendv};

/// `_IOMGR_NETWORK`: the manager of a `_IO_MSG` to `io-sock`.
const IOMGR_NETWORK: u16 = 6;

/// A `_IO_MSG` to `io-sock`: `combine_len`, and `subtype`, one of those
/// below.
pub const fn message(combine_len: u16, subtype: u16) -> Msg {
    Msg {
        r#type: IO_MSG,
        combine_len,
        mgrid: IOMGR_NETWORK,
        subtype,
    }
}

pub const MSG_SETOPT: u16 = 0;
pub const MSG_GETOPT: u16 = 1;
pub const MSG_SYSCTL: u16 = 3;

/// `struct iovec`, `struct msghdr` (`<sys/socket.h>`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Msghdr {
    pub name: *mut c_void,
    pub namelen: u32,
    pub iov: *mut Iov,
    pub iovlen: c_int,
    pub control: *mut c_void,
    pub controllen: u32,
    pub flags: c_int,
}

/// `struct cmsghdr`: its data starts `CMSG_HDR` bytes in.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Cmsghdr {
    pub len: u32,
    pub level: c_int,
    pub r#type: c_int,
}

pub const CMSG_HDR: usize = 16;
pub const SOL_SOCKET: c_int = 0xffff;
pub const SCM_RIGHTS: c_int = 1;

/// `_ALIGN()`: the alignment of control messages.
pub const fn align(n: usize) -> usize {
    (n + 7) & !7
}

/// A request to socket `fd`: the reply's status, or QNX's error; a
/// server that knows no such request is not a socket.
///
/// # Safety
///
/// Each part is valid for its direction.
pub unsafe fn request(fd: c_int, s: &[Iov], r: &[Iov]) -> Result<i64, c_int> {
    // SAFETY: the caller's guarantee.
    unsafe { msg_sendv(fd, s, r) }.map_err(|e| if e == ENOSYS { ENOTSOCK } else { e })
}

/// Room for `n` message parts: on the stack for a few, else mapped.
pub struct Parts {
    small: [Iov; Parts::SMALL],
    mapped: *mut Iov,
    n: usize,
}

impl Parts {
    const SMALL: usize = 16;

    /// Room for `n` parts; `Err(ENOMEM)` without memory.
    pub fn new(n: usize) -> Result<Parts, c_int> {
        let mut parts = Parts {
            small: [Iov::raw(core::ptr::null(), 0); Parts::SMALL],
            mapped: core::ptr::null_mut(),
            n,
        };
        if n > Parts::SMALL {
            parts.mapped = crate::mem::scratch(n * size_of::<Iov>())?.cast();
        }
        Ok(parts)
    }

    pub fn as_mut_ptr(&mut self) -> *mut Iov {
        if self.mapped.is_null() {
            self.small.as_mut_ptr()
        } else {
            self.mapped
        }
    }

    /// The first `count` parts.
    ///
    /// # Safety
    ///
    /// `count` of them were written, and at most `n`.
    pub unsafe fn slice(&self, count: usize) -> &[Iov] {
        let p = if self.mapped.is_null() { self.small.as_ptr() } else { self.mapped };
        // SAFETY: the caller's guarantee.
        unsafe { core::slice::from_raw_parts(p, count.min(self.n)) }
    }
}

impl Drop for Parts {
    fn drop(&mut self) {
        if !self.mapped.is_null() {
            crate::mem::unscratch(self.mapped.cast(), self.n * size_of::<Iov>());
        }
    }
}

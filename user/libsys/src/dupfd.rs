//! Descriptors passed between processes, as QNX's `dupfd_register()`,
//! `dupfd()` and `dupfd_unregister()` do it (libc, read for its
//! interface): the sender registers a descriptor for a process and gets
//! a handle (`ConnectDupFd`); the receiver looks the handle up, attaches
//! to the server and sends it an `_IO_DUP` with the sender's connection
//! info, which the server verifies with the kernel. io-sock passes
//! `SCM_RIGHTS` and `AF_UNIX` names so.

use core::ffi::c_int;
use core::ptr::null_mut;

use proto::info::MsgInfoHdr;
use proto::io::{Dup, IO_DUP};

use crate::kercall::{self, connect_attach, connect_detach, send};

const REGISTER: u32 = 1;
const REGISTER_DISC: u32 = 2;
const UNREGISTER: u32 = 3;
const LOOKUP: u32 = 4;
/// `DUPFD_NO_DISCARD`.
pub const NO_DISCARD: u32 = 1;
/// `FD_CLOEXEC`, and `_NTO_COF_CLOEXEC`.
const FD_CLOEXEC: c_int = 1;
const COF_CLOEXEC: c_int = 1;

/// Registers `fd` for process `pid` to take: the handle, or QNX's error.
pub fn register(pid: i32, fd: c_int, flags: u32) -> Result<u64, i32> {
    let mut dst = i64::from(pid);
    let mut handle = 0u64;
    let info = MsgInfoHdr {
        coid: fd,
        ..MsgInfoHdr::default()
    };
    let mut info = info;
    let op = if flags & NO_DISCARD != 0 { REGISTER } else { REGISTER_DISC };
    // SAFETY: all are on this stack.
    unsafe { kercall::connect_dupfd(op, &mut dst, &mut handle, null_mut(), &mut info) }?;
    Ok(handle)
}

/// Forgets `handle`.
pub fn unregister(handle: u64) {
    let mut h = handle;
    // SAFETY: the handle is on this stack; the rest may be null.
    let _ = unsafe { kercall::connect_dupfd(UNREGISTER, null_mut(), &mut h, null_mut(), null_mut()) };
}

/// Takes the descriptor `handle` stands for: a connection to its server
/// told of it with `_IO_DUP`. `flags`: `FD_CLOEXEC`.
pub fn take(handle: u64, flags: c_int) -> Result<c_int, i32> {
    let mut peer = 0i64;
    let mut h = handle;
    let mut srvpid = 0i32;
    let mut info = MsgInfoHdr::default();
    // SAFETY: all are on this stack.
    unsafe { kercall::connect_dupfd(LOOKUP, &mut peer, &mut h, &mut srvpid, &mut info) }?;
    let fd = connect_attach(0, srvpid, info.chid, 0, if flags & FD_CLOEXEC != 0 { COF_CLOEXEC } else { 0 })?;
    let msg = Dup {
        r#type: IO_DUP,
        combine_len: size_of::<Dup>() as u16,
        info,
        handle,
    };
    if let Err(e) = send(fd, &msg) {
        let _ = connect_detach(fd);
        return Err(e);
    }
    Ok(fd)
}

//! Descriptors passing through: QNX's `dupfd` protocol, as `libsys`'s
//! `dupfd.rs`, with `netd` as the receiver, then as the sender for whoever
//! reads the message.

use core::ffi::{c_int, c_void};
use core::ptr::null_mut;

use kercall::abi::Iov;
use kercall::neutrino::connect::{ConnectAttach_r, ConnectDetach_r, ConnectDupFd_r};
use kercall::neutrino::msg::MsgSendv_r;
use proto::info::MsgInfoHdr;
use proto::io::{Dup, IO_DUP};

const REGISTER_DISC: c_int = 2;
const LOOKUP: c_int = 4;
const COF_CLOEXEC: c_int = 1;

/// Takes the descriptor `handle` stands for: `netd`'s own copy.
pub fn import(handle: u64) -> Result<c_int, c_int> {
    let (mut peer, mut h, mut srvpid) = (0i64, handle, 0i32);
    let mut info = MsgInfoHdr::default();
    let e = ConnectDupFd_r(LOOKUP, (&raw mut peer).cast(), (&raw mut h).cast(), (&raw mut srvpid).cast(), (&raw mut info).cast());
    if e != 0 {
        return Err(e.abs());
    }
    let fd = ConnectAttach_r(0, srvpid, info.chid, 0, COF_CLOEXEC);
    if fd < 0 {
        return Err(-fd);
    }
    let msg = Dup { r#type: IO_DUP, combine_len: size_of::<Dup>() as u16, info, handle };
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Dup>())];
    let r = MsgSendv_r(fd, send.as_ptr().cast(), 1, null_mut::<c_void>().cast(), 0);
    if r < 0 {
        close(fd);
        return Err(-(r as c_int));
    }
    Ok(fd)
}

/// Registers `fd` for process `pid`: the handle, 0 if it can't.
pub fn register(pid: c_int, fd: c_int) -> u64 {
    let mut dst = i64::from(pid);
    let mut handle = 0u64;
    let mut info = MsgInfoHdr { coid: fd, ..MsgInfoHdr::default() };
    let mut srv = 0i32;
    let e = ConnectDupFd_r(REGISTER_DISC, (&raw mut dst).cast(), (&raw mut handle).cast(), (&raw mut srv).cast(), (&raw mut info).cast());
    if e == 0 { handle } else { 0 }
}

pub fn close(fd: c_int) {
    ConnectDetach_r(fd);
}

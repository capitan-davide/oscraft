//! QNX's kernel interface, `<sys/neutrino.h>`: the kernel calls, as C
//! functions for programs, and as Rust functions for the layers above.
//!
//! A program enters the kernel with a supervisor call; how is the
//! architecture's (`crate::arch`). What this module knows is the rest,
//! which is the same on every architecture: the calls, their numbers
//! (`<sys/kercalls.h>`), their prototypes, and how each reports a failure.
//! Each kernel call has up to two C functions: `Name()`, which returns -1
//! and sets `errno`, and `Name_r()`, which leaves `errno` alone and
//! returns the error instead, in one of two ways:
//!
//! - a call that returns a value (a connection ID, a byte count) returns
//!   the *negated* error number: `neg`;
//! - a call that returns only whether it worked returns EOK (0) or the
//!   error number itself: `status`.
//!
//! Which call uses which is QNX's ABI, not a choice: the table follows
//! QNX's library. Some calls also have variants that differ in their
//! arguments only: `MsgSend()` is `MsgSendv()` with both part counts
//! negated, which tells the kernel the parts are plain buffers.
//!
//! The calls are grouped by what they act on, one module each.

use core::ffi::{c_int, c_long};

use crate::abi::Iov;

/// Defines one C function of a kernel call: its convention (`errno`,
/// `neg` or `status`, above), its name, its number, the arguments it
/// negates, and its prototype. The function is naked: its body is the
/// architecture's few instructions, with the arguments still where the
/// caller put them.
macro_rules! kercall {
    ($(#[$doc:meta])* $conv:ident $name:ident = $nr:path $(, neg($($n:literal),+))?;
     fn($($arg:ident: $ty:ty),*) -> $ret:ty) => {
        $(#[$doc])*
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        pub extern "C" fn $name($($arg: $ty),*) -> $ret {
            crate::arch::kercall_asm!($conv, $nr, crate::neutrino::ker_err $(, neg($($n),+))?)
        }
    };
}

pub mod channel;
pub mod clock;
pub mod connect;
pub mod interrupt;
pub mod misc;
pub mod msg;
pub mod nr;
pub mod sched;
pub mod signal;
pub mod sync;
pub mod thread;
pub mod timer;

/// The failure path of every `Name()` kernel call: store the error number
/// the kernel returned in `errno`, and return -1.
pub(crate) extern "C" fn ker_err(error: c_int) -> c_long {
    crate::tls::set_errno(error);
    -1
}

/// `MsgSendv()` for the library's own use: the server's status, or the
/// error number. It uses the `_r` form, so that `errno` is left for the
/// C function that called it to set, or not.
///
/// # Safety
///
/// Each part of `send` and `reply` is valid for its length; the reply's
/// parts are writable.
pub unsafe fn msg_sendv(coid: c_int, send: &[Iov], reply: &[Iov]) -> Result<c_long, c_int> {
    let status = msg::MsgSendv_r(
        coid,
        send.as_ptr().cast(),
        send.len(),
        reply.as_ptr().cast(),
        reply.len(),
    );
    if status < 0 {
        Err(-status as c_int)
    } else {
        Ok(status)
    }
}

/// A message of one buffer, without the kernel's cancellation point
/// (`MsgSendnc()`): for requests a thread must not be cancelled in, such
/// as the memory manager's. The reply may go into the request's buffer.
///
/// # Safety
///
/// `send` is valid for `send_len` bytes, `reply` writable for
/// `reply_len`.
pub unsafe fn msg_sendvnc(
    coid: c_int,
    send: *const core::ffi::c_void,
    send_len: usize,
    reply: *mut core::ffi::c_void,
    reply_len: usize,
) -> Result<c_long, c_int> {
    let status = msg::MsgSendnc_r(coid, send, send_len, reply, reply_len);
    if status < 0 {
        Err(-status as c_int)
    } else {
        Ok(status)
    }
}

/// Sends `msg`, a message of [`proto`]'s, on `coid`, not as a
/// cancellation point, with no reply data: the server's status.
pub fn send_nc<T: proto::pod::Pod>(coid: c_int, msg: &T) -> Result<c_long, c_int> {
    // SAFETY: the message is valid for its size; there is no reply.
    unsafe {
        msg_sendvnc(
            coid,
            (msg as *const T).cast(),
            size_of::<T>(),
            core::ptr::null_mut(),
            0,
        )
    }
}

/// Sends `msg` on `coid`, not as a cancellation point, and returns the
/// server's status and its reply, a `R`.
pub fn ask_nc<T: proto::pod::Pod, R: proto::pod::Pod>(
    coid: c_int,
    msg: &T,
) -> Result<(c_long, R), c_int> {
    let mut reply = R::zeroed();
    // SAFETY: both are valid for their sizes; any bytes are an R.
    let status = unsafe {
        msg_sendvnc(
            coid,
            (msg as *const T).cast(),
            size_of::<T>(),
            (&raw mut reply).cast(),
            size_of::<R>(),
        )
    }?;
    Ok((status, reply))
}

/// [`ask_nc`], as a cancellation point.
pub fn ask<T: proto::pod::Pod, R: proto::pod::Pod>(
    coid: c_int,
    msg: &T,
) -> Result<(c_long, R), c_int> {
    let mut reply = R::zeroed();
    let send = [crate::abi::Iov::of(msg)];
    let back = [crate::abi::Iov::of_mut(&mut reply)];
    // SAFETY: both parts are valid for their sizes.
    let status = unsafe { msg_sendv(coid, &send, &back) }?;
    Ok((status, reply))
}

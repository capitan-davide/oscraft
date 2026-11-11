//! Control messages. Descriptors do not cross as numbers: `SCM_RIGHTS`
//! carries `dupfd` handles, 8 bytes each going out, and 16 coming in,
//! the handle last.

use core::ffi::{c_int, c_void};
use core::ptr::null_mut;

use proto::errno::{EAGAIN, EINVAL, EMFILE};

use super::{CMSG_HDR, Cmsghdr, SCM_RIGHTS, SOL_SOCKET, align};
use crate::dupfd;

/// A descriptor as it comes in, and the handle in it.
const WIRE_FD: usize = 16;
const WIRE_HANDLE: usize = 8;
/// `FD_CLOEXEC`, as `dupfd` takes it, and the flag that asks for it.
const FD_CLOEXEC: c_int = 1;
const MSG_CMSG_CLOEXEC: c_int = 0x4_0000;

/// Copies out the header at `p`, if a whole one is before `end`.
///
/// # Safety
///
/// `p` to `end` is readable.
unsafe fn header(p: *const u8, end: *const u8) -> Option<Cmsghdr> {
    if (end as usize) < p as usize + size_of::<Cmsghdr>() {
        return None;
    }
    // SAFETY: the caller's guarantee, and it is in bounds.
    Some(unsafe { p.cast::<Cmsghdr>().read_unaligned() })
}

fn is_rights(h: Cmsghdr) -> bool {
    h.level == SOL_SOCKET && h.r#type == SCM_RIGHTS
}

/// The bytes the wire form of the descriptors in `h` holds them in.
fn rights_len(h: Cmsghdr, each: usize) -> usize {
    (h.len as usize).saturating_sub(CMSG_HDR) / each
}

/// The wire form of control data: the caller's, or ours in a scratch
/// mapping when `SCM_RIGHTS` had to become handles.
pub struct Wire {
    pub ptr: *mut c_void,
    pub len: u32,
    mapped: usize,
}

impl Wire {
    pub fn none() -> Wire {
        Wire { ptr: null_mut(), len: 0, mapped: 0 }
    }
}

/// The control data of a message with `SCM_RIGHTS` as handles for server
/// `pid`, and its length; or as it is, without any. `Err` with QNX's
/// error. [`release()`] takes the result back.
///
/// # Safety
///
/// `control` is readable for `len` bytes.
pub unsafe fn to_wire(pid: i32, control: *const c_void, len: u32) -> Result<Wire, i32> {
    let mut out = Wire { ptr: control.cast_mut(), len, mapped: 0 };
    if control.is_null() || (len as usize) < size_of::<Cmsghdr>() {
        out.len = 0;
        return Ok(out);
    }
    let start = control.cast::<u8>();
    let end = start.wrapping_add(len as usize);
    let (mut p, mut size, mut any) = (start, 0, false);
    // SAFETY: p stays in the caller's bounds.
    while let Some(h) = unsafe { header(p, end) } {
        if (h.len as usize) < size_of::<Cmsghdr>() {
            return Err(EINVAL);
        }
        if is_rights(h) {
            any = true;
            size += CMSG_HDR + rights_len(h, 4) * 8;
        } else {
            size += align(h.len as usize);
        }
        p = p.wrapping_add(align(h.len as usize));
        if p >= end {
            break;
        }
    }
    if !any {
        return Ok(out);
    }
    let wire = crate::mem::scratch(size)?;
    // SAFETY: a fresh mapping of `size` bytes.
    unsafe { wire.write_bytes(0, size) };
    out = Wire { ptr: wire.cast(), len: size as u32, mapped: size };
    let (mut p, mut q) = (start, wire);
    // SAFETY: as above.
    while let Some(mut h) = unsafe { header(p, end) } {
        let taken = align(h.len as usize);
        let to = if is_rights(h) {
            let n = rights_len(h, 4);
            h.len = (CMSG_HDR + n * 8) as u32;
            for i in 0..n {
                // SAFETY: within the header's length, an int each.
                let fd = unsafe { p.wrapping_add(CMSG_HDR + i * 4).cast::<c_int>().read_unaligned() };
                match dupfd::register(pid, fd, dupfd::NO_DISCARD) {
                    Ok(handle) => {
                        // SAFETY: in the mapping, counted above.
                        unsafe { q.wrapping_add(CMSG_HDR + i * 8).cast::<u64>().write_unaligned(handle) };
                    }
                    Err(e) => {
                        // SAFETY: what was made so far.
                        unsafe { release(&out, control) };
                        return Err(if e == EAGAIN { EMFILE } else { e });
                    }
                }
            }
            0
        } else {
            (h.len as usize).min(end as usize - p as usize)
        };
        // SAFETY: the data of the source, and room in the count above.
        unsafe { q.copy_from_nonoverlapping(p, to) };
        // SAFETY: the header, in the count above.
        unsafe { q.cast::<Cmsghdr>().write_unaligned(h) };
        q = q.wrapping_add(align(h.len as usize));
        p = p.wrapping_add(taken);
        if p >= end {
            break;
        }
    }
    Ok(out)
}

/// Withdraws the handles of what [`to_wire()`] made, and frees it.
///
/// # Safety
///
/// `wire` is its result, for the `control` it was given.
pub unsafe fn release(wire: &Wire, control: *const c_void) {
    if wire.ptr.is_null() || wire.ptr.cast_const() == control || wire.mapped == 0 {
        return;
    }
    let start = wire.ptr.cast::<u8>();
    let end = start.wrapping_add(wire.len as usize);
    let mut p = start;
    // SAFETY: within the mapping.
    while let Some(h) = unsafe { header(p, end) } {
        if h.len == 0 {
            break;
        }
        if is_rights(h) {
            for i in 0..rights_len(h, 8) {
                // SAFETY: the handles of the message, zero if unmade.
                let handle = unsafe { p.wrapping_add(CMSG_HDR + i * 8).cast::<u64>().read_unaligned() };
                if handle != 0 {
                    dupfd::unregister(handle);
                }
            }
        }
        p = p.wrapping_add(align(h.len as usize));
    }
    crate::mem::unscratch(start, wire.mapped);
}

/// The wire form `wire` of a received message's control data for the
/// caller, in `dst`: descriptors taken up. How many bytes it wrote, and
/// whether any did not fit; `Err` with QNX's error, none kept.
///
/// # Safety
///
/// `wire` is readable for `len` bytes, `dst` writable for `cap`.
pub unsafe fn from_wire(wire: *const u8, len: usize, dst: *mut u8, cap: usize, flags: c_int) -> Result<(usize, bool), i32> {
    let end = wire.wrapping_add(len);
    let (mut p, mut q, mut room) = (wire, dst, cap);
    let (mut left, mut cut) = (len, false);
    let fd_flags = if flags & MSG_CMSG_CLOEXEC != 0 { FD_CLOEXEC } else { 0 };
    // SAFETY: the caller's bounds.
    while let Some(h) = unsafe { header(p, end) } {
        let taken = align(h.len as usize).max(size_of::<Cmsghdr>()).min(left);
        if room < size_of::<Cmsghdr>() {
            break;
        }
        if !is_rights(h) {
            let n = taken.min(room);
            cut |= n < taken;
            // SAFETY: the bytes are in both bounds.
            unsafe { q.copy_from_nonoverlapping(p, n) };
            q = q.wrapping_add(n);
            room -= n;
        } else {
            let head = CMSG_HDR.min(room);
            // SAFETY: as above.
            unsafe { q.copy_from_nonoverlapping(p, head) };
            let mut out = Cmsghdr { len: head as u32, ..h };
            let fds = q.wrapping_add(CMSG_HDR).cast::<c_int>();
            room -= head;
            for i in 0..rights_len(h, WIRE_FD) {
                if room < 4 {
                    cut = true;
                    break;
                }
                // SAFETY: a handle, in the header's length, in bounds.
                let handle = unsafe { p.wrapping_add(CMSG_HDR + i * WIRE_FD + WIRE_HANDLE).cast::<u64>().read_unaligned() };
                match dupfd::take(handle, fd_flags) {
                    Ok(fd) => {
                        // SAFETY: room for the int, checked.
                        unsafe { fds.wrapping_add(i).write_unaligned(fd) };
                        out.len += 4;
                        room -= 4;
                    }
                    Err(e) => {
                        let written = q as usize - dst as usize + out.len as usize;
                        // SAFETY: the header in dst, as far as it was written.
                        unsafe { q.cast::<Cmsghdr>().write_unaligned(out) };
                        // SAFETY: what this call wrote.
                        unsafe { close_rights(dst, written) };
                        return Err(e);
                    }
                }
            }
            // SAFETY: the header in dst.
            unsafe { q.cast::<Cmsghdr>().write_unaligned(out) };
            let space = align(out.len as usize);
            let pad = space - out.len as usize;
            q = q.wrapping_add(space.min(out.len as usize + room));
            room -= pad.min(room);
        }
        p = p.wrapping_add(taken);
        left -= taken;
        if left == 0 {
            break;
        }
    }
    Ok((q as usize - dst as usize, cut || left != 0))
}

/// Closes the descriptors in the control data at `control`.
///
/// # Safety
///
/// `control` is readable for `len` bytes.
pub unsafe fn close_rights(control: *const u8, len: usize) {
    let end = control.wrapping_add(len);
    let mut p = control;
    // SAFETY: as above.
    while let Some(h) = unsafe { header(p, end) } {
        if h.len == 0 {
            break;
        }
        if is_rights(h) {
            for i in 0..rights_len(h, 4) {
                // SAFETY: the ints of the header.
                let _ = crate::fs::__sys_close(unsafe { p.wrapping_add(CMSG_HDR + i * 4).cast::<c_int>().read_unaligned() });
            }
        }
        p = p.wrapping_add(align(h.len as usize));
    }
}

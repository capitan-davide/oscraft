//! Receiving: `recvfrom()` and `recvmsg()`.

use core::ffi::{c_int, c_void};

use proto::errno::{EILSEQ, EMSGSIZE};
use proto::io::{IO_READ, IO_READ64, ReadWrite};

use super::control::{close_rights, from_wire};
use super::transmit::XTYPE_TCPIP;
use super::{Msghdr, Parts, request};
use crate::errno::fail;
use crate::kercall::Iov;

/// `_IO_XTYPE_TCPIP_MSG2`.
const XTYPE_TCPIP_MSG2: u32 = 7;
/// The longest name `recvfrom()` asks for.
const NAME_MAX: u32 = 255;
const MSG_PEEK: c_int = 2;
const MSG_CTRUNC: c_int = 0x20;

/// `io_sock_recvfrom_t`.
#[repr(C)]
struct Recvfrom {
    read: ReadWrite,
    flags: c_int,
    addrlen: u32,
}

/// `io_sock_recvmsg_t`.
#[repr(C)]
struct Recvmsg {
    read: ReadWrite,
    flags: c_int,
    addrlen: u32,
    controllen: u32,
    controlseq: u32,
}

/// Its reply.
#[repr(C)]
#[derive(Default)]
struct RecvmsgReply {
    flags: c_int,
    addrlen: u32,
    controllen: u32,
    controltot: u32,
    controlseq: u32,
}

/// `recvfrom(fd, buf, len, flags, from, fromlen)`.
///
/// # Safety
///
/// `buf` is writable for `len` bytes; `from` and `fromlen` are null or
/// valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_recvfrom(fd: c_int, buf: *mut c_void, len: usize, flags: c_int, from: *mut c_void, fromlen: *mut u32) -> isize {
    let wanted = if from.is_null() || fromlen.is_null() {
        0
    } else {
        // SAFETY: the caller's guarantee.
        unsafe { *fromlen }.min(NAME_MAX)
    };
    let msg = Recvfrom {
        read: ReadWrite {
            r#type: if len >> 32 == 0 { IO_READ } else { IO_READ64 },
            combine_len: size_of::<Recvfrom>() as u16,
            nbytes: len as u32,
            xtype: XTYPE_TCPIP,
            nbytes_hi: (len >> 32) as u32,
        },
        flags,
        addrlen: wanted,
    };
    let mut reply = 0u32;
    let out = [
        Iov::raw((&raw mut reply).cast(), 4),
        Iov::raw(from.cast_const(), wanted as usize),
        Iov::raw(buf.cast_const(), len),
    ];
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Recvfrom>())];
    // SAFETY: the request and the caller's buffers.
    match unsafe { request(fd, &send, &out) } {
        Ok(n) => {
            if !fromlen.is_null() && wanted != 0 {
                // SAFETY: the caller's guarantee.
                unsafe { *fromlen = reply };
            }
            n as isize
        }
        Err(e) => fail(e),
    }
}

/// The request of a message.
fn request_of(flags: c_int, nbytes: u32, addrlen: u32, controllen: u32, seq: u32) -> Recvmsg {
    Recvmsg {
        read: ReadWrite {
            r#type: IO_READ,
            combine_len: size_of::<Recvmsg>() as u16,
            nbytes,
            xtype: XTYPE_TCPIP_MSG2,
            nbytes_hi: 0,
        },
        flags,
        addrlen,
        controllen,
        controlseq: seq,
    }
}

/// The control data a message carries: peeked, as io-sock sends it
/// (descriptors as handles, 16 bytes each), into a scratch mapping:
/// (mapping, its size, the data's length, the sequence number that
/// names the data for the read that takes it).
///
/// # Safety
///
/// `m` is a valid message with control data.
unsafe fn peek_control(fd: c_int, m: &Msghdr, addrlen: u32, flags: c_int) -> Result<(*mut u8, usize, usize, u32), i32> {
    let mut reply = RecvmsgReply::default();
    let mut one = 0u8;
    let peek = flags | MSG_PEEK;
    let msg = request_of(peek, 1, addrlen, m.controllen, 0);
    let parts = [
        Iov::raw((&raw mut reply).cast(), size_of::<RecvmsgReply>()),
        Iov::raw(m.name.cast_const(), addrlen as usize),
        Iov::raw(m.control.cast_const(), m.controllen as usize),
        Iov::raw((&raw mut one).cast(), 1),
    ];
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Recvmsg>())];
    // SAFETY: the request, and the caller's buffers.
    unsafe { request(fd, &send, &parts) }?;
    let (total, seq) = (reply.controltot as usize, reply.controlseq);
    let size = total.max(1);
    let wire = crate::mem::scratch(size)?;
    if reply.flags & MSG_CTRUNC == 0 {
        // SAFETY: the reply wrote that much in the caller's buffer.
        unsafe { wire.copy_from_nonoverlapping(m.control.cast(), (reply.controllen as usize).min(total)) };
        return Ok((wire, size, total, seq));
    }
    let msg = request_of(peek, 1, addrlen, total as u32, seq);
    let parts = [
        Iov::raw((&raw mut reply).cast(), size_of::<RecvmsgReply>()),
        Iov::raw(m.name.cast_const(), addrlen as usize),
        Iov::raw(wire.cast_const().cast(), total),
        Iov::raw((&raw mut one).cast(), 1),
    ];
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Recvmsg>())];
    // SAFETY: as above, with the new buffer.
    if let Err(e) = unsafe { request(fd, &send, &parts) } {
        crate::mem::unscratch(wire, size);
        return Err(e);
    }
    Ok((wire, size, total, seq))
}

/// `recvmsg(fd, msg, flags)`. Descriptors in control data are taken up
/// before the message goes, so they cannot be lost; if another reader
/// took the message first, EILSEQ says to begin again.
///
/// # Safety
///
/// `m` is a valid message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_recvmsg(fd: c_int, m: *mut Msghdr, flags: c_int) -> isize {
    // SAFETY: the caller's guarantee.
    let hdr = unsafe { &mut *m };
    if hdr.iovlen < 1 {
        return fail(EMSGSIZE);
    }
    let addrlen = if hdr.name.is_null() { 0 } else { hdr.namelen.min(NAME_MAX) };
    let n = hdr.iovlen as usize;
    let mut parts = match Parts::new(n + 3) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    loop {
        // SAFETY: the parts are ours; the message is the caller's.
        match unsafe { read_once(fd, hdr, addrlen, flags, &mut parts) } {
            Err(EILSEQ) => continue,
            Err(e) => return fail(e),
            Ok(r) => return r,
        }
    }
}

/// One try of `recvmsg()`: QNX's error, or the bytes received.
///
/// # Safety
///
/// `parts` has room for the message's parts and 3 more.
unsafe fn read_once(fd: c_int, m: &mut Msghdr, addrlen: u32, flags: c_int, parts: &mut Parts) -> Result<isize, i32> {
    let with_control = !m.control.is_null() && m.controllen != 0;
    let (mut wire, mut size, mut total, mut seq) = (core::ptr::null_mut::<u8>(), 0usize, 0usize, 0u32);
    if with_control {
        // SAFETY: the caller's guarantee.
        (wire, size, total, seq) = unsafe { peek_control(fd, m, addrlen, flags) }?;
    }
    let (mut written, mut cut) = (0, false);
    if total != 0 {
        // SAFETY: what the peek filled, and the caller's buffer.
        match unsafe { from_wire(wire, total, m.control.cast(), m.controllen as usize, flags) } {
            Ok(done) => (written, cut) = done,
            Err(e) => {
                crate::mem::unscratch(wire, size);
                return Err(e);
            }
        }
    }
    let n = m.iovlen as usize;
    let iov = parts.as_mut_ptr();
    let mut reply = RecvmsgReply::default();
    let mut count = 0usize;
    let mut nbytes = 0usize;
    // SAFETY: room for n + 3 parts; the message's parts are the caller's.
    unsafe {
        iov.write(Iov::raw((&raw mut reply).cast(), size_of::<RecvmsgReply>()));
        count += 1;
        if addrlen != 0 {
            iov.add(count).write(Iov::raw(m.name.cast_const(), addrlen as usize));
            count += 1;
        }
        if with_control {
            iov.add(count).write(Iov::raw(wire.cast_const().cast(), total));
            count += 1;
        }
        for i in 0..n {
            let part = m.iov.add(i).read();
            iov.add(count).write(part);
            count += 1;
            nbytes += part.len;
        }
    }
    let msg = request_of(flags, nbytes as u32, addrlen, total as u32, seq);
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Recvmsg>())];
    // SAFETY: `count` parts were written.
    let list = unsafe { parts.slice(count) };
    // SAFETY: each part is a buffer of the caller's, or ours.
    let r = unsafe { request(fd, &send, list) };
    if with_control {
        crate::mem::unscratch(wire, size);
    }
    match r {
        Ok(r) => {
            m.flags = if cut { reply.flags | MSG_CTRUNC } else { reply.flags };
            if !m.name.is_null() {
                m.namelen = reply.addrlen;
            }
            m.controllen = written as u32;
            Ok(r as isize)
        }
        Err(e) => {
            // SAFETY: what from_wire() wrote.
            unsafe { close_rights(m.control.cast_const().cast(), written) };
            Err(e)
        }
    }
}

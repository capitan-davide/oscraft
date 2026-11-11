//! Memory: `mmap()` and its family, as messages to the memory manager,
//! which is the process manager (`<sys/memmsg.h>`: `_MEM_MAP`,
//! `_MEM_CTRL`). The protection and mapping flags are FreeBSD's on the
//! way in (`<sys/mman.h>`) and QNX's in the message.

use core::ffi::{c_int, c_void};

use proto::mem::{self as m, Ctrl, Map, MapReply};

use crate::errno::fail;
use crate::kercall::{SYSMGR_COID, ask, send};

// FreeBSD's flags.
const BSD_PROT_READ: c_int = 1;
const BSD_PROT_WRITE: c_int = 2;
const BSD_PROT_EXEC: c_int = 4;
const BSD_MAP_SHARED: c_int = 0x1;
const BSD_MAP_PRIVATE: c_int = 0x2;
const BSD_MAP_FIXED: c_int = 0x10;
const BSD_MAP_STACK: c_int = 0x400;
const BSD_MAP_ANON: c_int = 0x1000;
const BSD_MAP_GUARD: c_int = 0x2000;
const BSD_MAP_ALIGNMENT_SHIFT: u32 = 24;
// QNX's.
const QNX_PROT_READ: u32 = 0x100;
const QNX_PROT_WRITE: u32 = 0x200;
const QNX_PROT_EXEC: u32 = 0x400;
const QNX_PROT_NONE: u32 = 0;
const QNX_MAP_SHARED: u32 = 0x1;
const QNX_MAP_PRIVATE: u32 = 0x2;
const QNX_MAP_FIXED: u32 = 0x10;
const QNX_MAP_STACK: u32 = 0x1000;
const QNX_MAP_ANON: u32 = 0x8_0000;

fn qnx_prot(prot: c_int) -> u32 {
    let mut q = QNX_PROT_NONE;
    if prot & BSD_PROT_READ != 0 {
        q |= QNX_PROT_READ;
    }
    if prot & BSD_PROT_WRITE != 0 {
        q |= QNX_PROT_WRITE;
    }
    if prot & BSD_PROT_EXEC != 0 {
        q |= QNX_PROT_EXEC;
    }
    q
}

/// `mmap(addr, len, prot, flags, fd, offset)`: a `_MEM_MAP` request.
/// `MAP_ALIGNED(n)` becomes the request's alignment; `MAP_GUARD`, a
/// reservation FreeBSD backs with nothing, is a `PROT_NONE` anonymous
/// mapping here (QNX's memory is always backed).
///
/// # Safety
///
/// As `mmap()`: `MAP_FIXED` replaces what was at `addr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mmap(
    addr: *mut c_void,
    len: usize,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: i64,
) -> *mut c_void {
    let bsd_prot = prot;
    let align_log2 = (flags as u32) >> BSD_MAP_ALIGNMENT_SHIFT;
    // ANCHOR: mmap_flags
    let mut q = 0;
    for (bsd, qnx) in [
        (BSD_MAP_SHARED, QNX_MAP_SHARED),
        (BSD_MAP_PRIVATE, QNX_MAP_PRIVATE),
        (BSD_MAP_FIXED, QNX_MAP_FIXED),
        (BSD_MAP_STACK, QNX_MAP_STACK),
        (BSD_MAP_ANON, QNX_MAP_ANON),
    ] {
        if flags & bsd != 0 {
            q |= qnx;
        }
    }
    // FreeBSD's MAP_STACK is anonymous memory, and MAP_ANON alone is a
    // private mapping; QNX wants both said.
    if flags & BSD_MAP_STACK != 0 {
        q |= QNX_MAP_ANON;
    }
    if flags & (BSD_MAP_ANON | BSD_MAP_STACK) != 0 && flags & BSD_MAP_SHARED == 0 {
        q |= QNX_MAP_PRIVATE;
    }
    let (prot, fd) = if flags & BSD_MAP_GUARD != 0 {
        q |= QNX_MAP_ANON | QNX_MAP_PRIVATE;
        (QNX_PROT_NONE, -1)
    } else {
        (qnx_prot(prot), fd)
    };
    // ANCHOR_END: mmap_flags
    let msg = Map {
        r#type: m::MEM_MAP,
        addr: addr as u64,
        len: len as u64,
        prot,
        flags: q,
        fd,
        align: if align_log2 != 0 { 1u64 << align_log2 } else { 0 },
        offset,
        ..Map::default()
    };
    match ask::<Map, MapReply>(SYSMGR_COID, &msg) {
        Ok((_, reply)) => reply.addr as *mut c_void,
        // SAFETY: the arguments as the caller gave them.
        Err(proto::errno::ENODEV) if fd >= 0 => unsafe { map_by_copy(addr, len, bsd_prot, q, fd, offset as u64) },
        Err(e) => fail(e) as *mut c_void,
    }
}

/// A file on another server, which the memory manager can't reach: a copy
/// of it, read now. A mapping is fully backed when it is made, as QNX 8's
/// are, and a stuck server holds up this thread alone. Changes can't go
/// back to the file, so a writable shared mapping is refused.
///
/// # Safety
///
/// As `mmap()`.
unsafe fn map_by_copy(addr: *mut c_void, len: usize, prot: c_int, flags: u32, fd: c_int, offset: u64) -> *mut c_void {
    const PROT_READ_WRITE: c_int = 3;
    const PROT_WRITE: c_int = 2;
    const BSD_MAP_FIXED: c_int = 0x10;
    if flags & QNX_MAP_PRIVATE == 0 && prot & PROT_WRITE != 0 {
        return fail(proto::errno::ENODEV) as *mut c_void;
    }
    let fixed = if flags & QNX_MAP_FIXED != 0 { BSD_MAP_FIXED } else { 0 };
    // SAFETY: an anonymous mapping, placed as the caller asked.
    let at = unsafe { __sys_mmap(addr, len, PROT_READ_WRITE, BSD_MAP_PRIVATE | BSD_MAP_ANON | fixed, -1, 0) };
    if at as isize == -1 {
        return at;
    }
    let mut done = 0;
    while done < len {
        // SAFETY: inside the mapping just made.
        let piece = unsafe { at.cast::<u8>().add(done) };
        // SAFETY: as above, for what is left of it.
        let n = unsafe { crate::fd::__sys_pread(fd, piece.cast(), len - done, (offset + done as u64) as i64) };
        match n {
            ..0 => {
                __sys_munmap(at, len);
                return -1isize as *mut c_void;
            }
            0 => break,
            n => done += n as usize,
        }
    }
    __sys_mprotect(at, len, prot);
    at
}

/// A `_MEM_CTRL` request on `len` bytes at `addr`.
fn control(subtype: u16, flags: u32, addr: *mut c_void, len: usize) -> c_int {
    let msg = Ctrl {
        r#type: m::MEM_CTRL,
        subtype,
        flags,
        addr: addr as u64,
        len: len as u64,
    };
    match send(SYSMGR_COID, &msg) {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `munmap(addr, len)`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_munmap(addr: *mut c_void, len: usize) -> c_int {
    control(m::CTRL_UNMAP, 0, addr, len)
}

/// `mprotect(addr, len, prot)`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int {
    control(m::CTRL_PROTECT, qnx_prot(prot), addr, len)
}

/// `madvise(addr, len, behav)`: advice only, and QNX's memory is backed
/// whatever the advice, so it is taken and nothing is done.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_madvise(_addr: *mut c_void, _len: usize, _behav: c_int) -> c_int {
    0
}

/// `minherit(addr, len, inherit)`: how a mapping passes to a forked
/// child. A fork copies every mapping here, which is the default
/// (`INHERIT_COPY`); the other modes are accepted and not kept.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_minherit(_addr: *mut c_void, _len: usize, _inherit: c_int) -> c_int {
    0
}

/// Scratch memory of `len` bytes for a call that outgrows its stack
/// (libsys has no heap): an anonymous mapping, `unscratch`ed after.
pub fn scratch(len: usize) -> Result<*mut u8, c_int> {
    const PROT_RW: c_int = 3;
    const MAP_PRIVATE_ANON: c_int = 0x1002;
    // SAFETY: a new anonymous mapping of the process's own.
    let p = unsafe { __sys_mmap(core::ptr::null_mut(), len, PROT_RW, MAP_PRIVATE_ANON, -1, 0) };
    if p as isize == -1 {
        return Err(proto::errno::ENOMEM);
    }
    Ok(p.cast())
}

/// Gives [`scratch`]'s memory back.
pub fn unscratch(p: *mut u8, len: usize) {
    let _ = __sys_munmap(p.cast(), len);
}

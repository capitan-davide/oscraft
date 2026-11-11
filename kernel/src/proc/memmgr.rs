//! procnto's memory manager messages (`<sys/memmsg.h>`), received on the
//! system-manager channel: `_MEM_MAP` (`mmap()`) and `_MEM_CTRL`
//! (`munmap()`, `mprotect()`, `msync()`, `mlock()`, `madvise()`).
//!
//! Contract, from the SDP headers and procnto's `memmgr_map` and
//! `memmgr_ctrl`:
//! - `_MEM_MAP` needs the whole 72-byte `struct _mem_map` (else EBADMSG).
//!   The share type (`flags & MAP_TYPE`) must be `MAP_SHARED` or
//!   `MAP_PRIVATE`, `len` non-zero and `prot` only `PROT_*` bits (else
//!   EINVAL). `fd == NOFD` means anonymous memory. The reply,
//!   `struct _mem_map_reply`, overlays the request: `real_size` (mapped size
//!   plus the address's offset in its page), `real_addr` (page-aligned
//!   start) and `addr` (the address `mmap()` returns).
//!   Flags it doesn't know (`0xfff00700`) are EINVAL. With `MAP_FIXED` the
//!   address needn't be page-aligned: its page is mapped and the address
//!   itself returned.
//! - `_MEM_CTRL`: `addr + len` overflowing clamps `len` to the end of the
//!   address space; the range is rounded out to pages.
//!   `_MEM_CTRL_UNMAP` needs `len != 0`, `_MEM_CTRL_PROTECT` takes the new
//!   `PROT_*` bits in `flags`; `msync()` at most one of `MS_SYNC`,
//!   `MS_ASYNC`, `MS_SYNC_FULL`; `mlock()` a mapped range (ENOMEM),
//!   `mlockall()` flags 1 to 3, `madvise()` advice 0 to 5 (else EINVAL).
//!
//! - `_MEM_MAP_INFO` { type, subtype, reserved, `pid64_t` pid, startaddr,
//!   endaddr } (introspection, `memmgr_map_info`): subtype 0 lists the
//!   regions from `startaddr` to `endaddr` as `struct _procfs_map_info`s,
//!   the status their size in bytes; subtype 3 gives the
//!   `procfs_debuginfo` (object's lowest address, path) of the object
//!   mapped at `startaddr` (`/dev/zero` for anonymous memory, ENXIO if
//!   nothing is mapped there), the status its size. pidin only counts
//!   regions with an inode, and RAM's (`MAP_SYSRAM`) as data.
//!   Subtypes 1 and 2 (physical memory, page tables) are not implemented.
//! - `_MEM_INFO` { type, zero, fd, flags, reserved } (`mem_get_info()`,
//!   `memmgr_info`): with `fd` `NOFD`, the reply { zero[2], size,
//!   reserved, `struct __posix_typed_mem_info64` } gives the free memory
//!   (`posix_tmi_length`) and all of it (`__posix_tmi_total`). Typed memory
//!   objects (other `fd`s) are not implemented: ENODEV, procnto's answer
//!   for a descriptor that isn't one.
//! - `_MEM_READ_MEM` (same message, `memmgr_read_mem`): as many bytes from
//!   `startaddr` as the reply buffer holds, up to the end of the region
//!   there (ESRVRFAULT if none); the status is the number of bytes.
//!
//! Anonymous memory and the image filesystem's files can be mapped, as
//! private copies (shared is the same as private until there is `fork()`,
//! and the image is read-only); everything stays resident,
//! so sync, lock and advice requests succeed without doing anything.

use crate::arch::kercall::call;
use crate::ker::errno::{EACCES, EBADMSG, EINVAL, ENODEV, ENOMEM, ENOSYS, ENXIO, ESRCH, EPERM};
use crate::ker::number::*;
use crate::ker::process::{self, Process};
use crate::ker::umem;
use crate::vm::map::{self, MAP_ANON, MAP_PRIVATE};
use crate::vm::pmem::PAGE_SIZE;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::vm::object::MemObject;

use super::{imagefs, resmgr};

pub const MEM_MAP: u16 = 0x0040;
/// `mmap()` flag: `shmem_handle` names the object (`mmap_handle()`).
const MAP_HANDLE: u32 = 0x4_0000;
/// The `munmap_flags()` flags procnto accepts (`_MEM_CTRL_UNMAP`).
const UNMAP_FLAGS: u32 = 0xf;
pub const MEM_CTRL: u16 = 0x0041;
pub const MEM_INFO: u16 = 0x0042;
pub const MEM_OFFSET: u16 = 0x0043;
pub const MEM_PEER: u16 = 0x0047;
/// The last `_MEM_OFFSET` subtype (`_MEM_OFFSET_PHYS`, `_FD`, `_PT`).
const OFFSET_PT: u16 = 2;
/// `sizeof(struct _mem_info_reply)`
const MEM_INFO_REPLY_SIZE: usize = 40;
pub const MEM_MAP_INFO: u16 = 0x004e;
pub const MEM_READ_MEM: u16 = 0x004f;
const MAP_INFO_VIRTUAL: u16 = 0;
const MAP_INFO_PHYSICAL: u16 = 1;
const MAP_INFO_PAGETABLE: u16 = 2;
const MAP_INFO_OBJECT: u16 = 3;
pub const MEM_DEBUG_INFO: u16 = 0x0044;
pub const MEM_PMEM_ADD: u16 = 0x0046;
pub const MEM_DEBUG_INFO64: u16 = 0x0049;
pub const MEM_OBJ_INFO: u16 = 0x004a;
pub const MEM_AS_INFO: u16 = 0x004d;
/// `sizeof(struct _debug_aspace)`: as_size, as_used, map_count,
/// map_private, map_shared, rss, reserved[10].
const DEBUG_ASPACE_SIZE: usize = 128;
/// `sizeof(struct _procfs_map_info)`
const PROCFS_MAP_INFO_SIZE: usize = 48;
const MAP_SYSRAM: u32 = 0x100_0000;
const MAP_LAZY: u32 = 0x80;
/// `MAP_STACK | _MAP_RESERVE11`: the main thread's stack (see `loader`).
const MAIN_STACK: u32 = 0x8000_1000;
/// `st_dev` of anonymous and shared memory, of the image filesystem, and
/// of physical memory (`/dev/mem`'s), in `procfs_mapinfo` (measured).
const ANON_DEV: u32 = 0x1;
const IFS_DEV: u32 = 0x802;
const PHYS_DEV: u32 = 0x407;
/// `ESRVRFAULT`
use crate::ker::errno::ESRVRFAULT;
/// `_MEM_CTRL` subtypes.
const CTRL_UNMAP: u16 = 0;
const CTRL_PROTECT: u16 = 1;
const CTRL_SYNC: u16 = 2;
const CTRL_LOCKALL: u16 = 3;
const CTRL_LOCK: u16 = 5;
const CTRL_ADVISE: u16 = 7;
/// `<sys/mman.h>`
const MAP_SHARED: u32 = 0x1;
const MAP_TYPE: u32 = 0x3;
const MAP_PHYS: u32 = 0x10000;
const PROT_MASK: u32 = 0xf00;

/// The pages a mapping of `len` bytes starting `in_page` bytes into its
/// first page covers, in bytes; `None` when the sum has no room in the
/// address arithmetic. A length near the top of the range is a program's
/// to send (review PATHMEM-6): procnto answers ENOMEM for anonymous,
/// shared and file mappings and EINVAL for physical ones, and EINVAL
/// where the file offset plus the length overflows (`offset_end`), all
/// measured.
fn span(len: u64, in_page: u64) -> Option<u64> {
    len.checked_add(in_page)?.checked_next_multiple_of(PAGE_SIZE)
}

/// The end of a file or object mapping's `len` bytes from `offset`:
/// EINVAL past the end of the arithmetic (procnto, measured).
fn offset_end(offset: u64, len: u64) -> Result<u64, u32> {
    offset.checked_add(len).ok_or(EINVAL)
}
const NOFD: i32 = -1;
/// `sizeof(struct _mem_map)`
const MEM_MAP_SIZE: usize = 72;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn error(rcvid: u64, errno: u32) {
    let _ = call(__KER_MSG_ERROR, [rcvid, errno as u64, 0, 0, 0, 0]);
}

/// The process a request is about: `pid` 0 means the sender.
fn target(sender: i32, pid: i32) -> Option<crate::ker::process::ProcessRef> {
    process::lookup(if pid == 0 { sender } else { pid })
}

fn mem_map(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < MEM_MAP_SIZE {
        return Err(EBADMSG);
    }
    let pid = u32_at(msg, 4) as i32;
    let (addr, len) = (u64_at(msg, 8), u64_at(msg, 16));
    let (prot, flags, fd) = (u32_at(msg, 24), u32_at(msg, 28), u32_at(msg, 32) as i32);
    let align = u64_at(msg, 40);
    // As procnto's `memmgr_map`: flags beyond those it knows are EINVAL,
    // and so is anonymous or physical memory asked of a descriptor
    // (measured; `MAP_PHYS | MAP_PRIVATE` with `NOFD` is accepted).
    const MAP_UNKNOWN: u32 = 0xfff0_0700;
    if !matches!(flags & MAP_TYPE, MAP_SHARED | MAP_PRIVATE)
        || len == 0
        || prot & !PROT_MASK != 0
        || flags & MAP_UNKNOWN != 0
        || (flags & (MAP_ANON | MAP_PHYS) != 0 && fd != NOFD)
    {
        return Err(EINVAL);
    }
    // mmap_handle(): a shared memory handle for the sender (`pid`, if
    // not 0, its creator), used up by this.
    if flags & MAP_HANDLE != 0 && fd == NOFD {
        let me = process::lookup(sender).ok_or(ESRCH)?;
        let (file, offset) =
            super::shmhandle::for_mmap(u64_at(msg, 56), sender, pid, prot, u64_at(msg, 48), len)?;
        let mut m = msg.to_vec();
        m[48..56].copy_from_slice(&offset.to_le_bytes());
        m[28..32].copy_from_slice(&(flags & !MAP_HANDLE).to_le_bytes());
        // The handle's own access was checked above; whether a later
        // mprotect() may exceed it wasn't measured: no cap.
        return mem_map_shm(rcvid, &me, &m, file.obj.clone(), file.revocable(), map::PROT_ALL);
    }
    let p = target(sender, pid).ok_or(ESRCH)?;
    // A fixed address needs MAP_FIXED, memory both writable and executable
    // PROT_WRITE_AND_EXEC (as procnto).
    if flags & map::MAP_FIXED != 0 && !crate::ker::ability::allowed(&p, crate::ker::ability::MAP_FIXED, Some(addr)) {
        return Err(EPERM);
    }
    if prot & map::PROT_WRITE != 0 && prot & map::PROT_EXEC != 0 && !crate::ker::ability::allowed(&p, crate::ker::ability::PROT_WRITE_AND_EXEC, Some(addr)) {
        return Err(EPERM);
    }
    let client = resmgr::Client {
        pid: sender,
        coid: fd,
    };
    if fd != NOFD
        && let Some((file, ioflag)) = resmgr::shm_file_ioflag(client)
    {
        // The descriptor's access mode bounds the mapping's (EACCES).
        super::shmem::map_allowed(ioflag, prot, flags & MAP_TYPE == MAP_SHARED)?;
        // A shared mapping of a descriptor not open for writing can't be
        // made writable later either (mprotect(): EACCES, measured).
        let max_prot = if flags & MAP_TYPE == MAP_SHARED && !super::shmem::writable(ioflag) {
            map::PROT_ALL & !map::PROT_WRITE
        } else {
            map::PROT_ALL
        };
        return mem_map_shm(rcvid, &p, msg, file.obj.clone(), file.revocable(), max_prot);
    }
    if fd != NOFD
        && let Some(ty) = resmgr::tymem_object(client)
    {
        return mem_map_tymem(rcvid, &p, msg, &ty);
    }
    let (contents, pg_off, object) = if fd == NOFD {
        if flags & MAP_PHYS != 0 && flags & MAP_ANON == 0 {
            // mmap_device_memory(): the offset is the physical address.
            return mem_map_phys(rcvid, &p, addr, len, prot, flags, u64_at(msg, 48));
        }
        // Memory for a device's DMA: MAP_PHYS | MAP_ANON is physically
        // contiguous RAM (below 4 GB, which every device can reach), and
        // so, here, is uncached anonymous memory.
        if flags & MAP_ANON != 0 && (flags & MAP_PHYS != 0 || prot & map::PROT_NOCACHE != 0) {
            let size = span(len, 0).ok_or(ENOMEM)?;
            // A failure is PROCMGR_EVENT_CONTIG_ALLOC_FAIL (procnto's
            // memmgr_map(): for MAP_PHYS | MAP_ANON).
            let pa = crate::vm::pmem::alloc_contig(size / PAGE_SIZE, 0, 0xffff_ffff).ok_or_else(|| {
                if flags & MAP_PHYS != 0 {
                    super::procevent::trigger(super::procevent::CONTIG_ALLOC_FAIL, 0);
                }
                ENOMEM
            })?;
            let start = map::map_contig(&p, addr - addr % PAGE_SIZE, size, prot, flags, pa)?;
            reply_mapped(rcvid, size, start, start);
            return Ok(());
        }
        // Shared anonymous memory: an anonymous memory object, so that a
        // fork()ed child shares it (POSIX), as with shm_open() memory.
        if flags & MAP_ANON != 0 && flags & MAP_TYPE == MAP_SHARED {
            let size = span(len, 0).ok_or(ENOMEM)?;
            let obj = Arc::new(MemObject::new());
            obj.set_size(size)?;
            let start = map::map_shared(&p, addr - addr % PAGE_SIZE, size, prot, flags, obj, 0, None, false, map::PROT_ALL)?;
            reply_mapped(rcvid, size, start, start);
            return Ok(());
        }
        // The offset's position in its page still places the address
        // (procnto).
        (&[][..], u64_at(msg, 48) % PAGE_SIZE, None)
    } else {
        // A file: so far only procnto's own (the image filesystem's).
        // QNX would ask other servers with _IO_MMAP.
        let (file, path) = resmgr::file_contents(client)?;
        if flags & MAP_TYPE == MAP_SHARED && prot & map::PROT_WRITE != 0 {
            return Err(EACCES); // read-only filesystem
        }
        let offset = u64_at(msg, 48);
        offset_end(offset, len)?;
        let pg_off = offset % PAGE_SIZE;
        let from = (offset - pg_off) as usize;
        (file.get(from..).unwrap_or(&[]), pg_off, Some(path))
    };
    // The mapping starts at the address's page; `mmap()` returns the
    // address itself with MAP_FIXED, else the offset's position in the
    // first page (procnto).
    let in_page = if flags & map::MAP_FIXED != 0 { addr % PAGE_SIZE } else { pg_off };
    let size = span(len, in_page).ok_or(ENOMEM)?;
    // The file offset of the first page (for introspection).
    let file_off = if object.is_some() { u64_at(msg, 48) - pg_off } else { 0 };
    // Code and read-only data of the image's files: mapped in place
    // (shared, not copied), as QNX runs them from the image. Private or
    // shared alike (the dynamic loader maps code shared): read-only, they
    // can't tell.
    if let Some(name) = object
        && prot & map::PROT_WRITE == 0
        && let Some(pa) = super::imagefs::phys_of(contents)
        && pa.is_multiple_of(PAGE_SIZE)
        && contents.len() as u64 >= size - PAGE_SIZE
    {
        let start = map::map_image(
            &p,
            addr - addr % PAGE_SIZE,
            size,
            prot,
            flags,
            align,
            pa,
            Some(name),
        )?;
        p.mem.lock().set_offset(start, file_off);
        reply_mapped(rcvid, size, start, start + in_page);
        return Ok(());
    }
    let start = map::map(
        &p,
        addr - addr % PAGE_SIZE,
        size,
        prot,
        flags,
        align,
        contents,
        object,
    )?;
    p.mem.lock().set_offset(start, file_off);
    // A thread's stack (libc's pthread_create()): MAP_ANON | MAP_STACK
    // with the guard's size as the offset; its first pages are the guard,
    // never accessible (as procnto: an overflow faults, SEGV_MAPERR).
    let guard = u64_at(msg, 48) & !(PAGE_SIZE - 1);
    if fd == NOFD && flags & map::MAP_STACK != 0 && guard != 0 && guard < size {
        map::make_guard(&p, start, guard);
    }
    reply_mapped(rcvid, size, start, start + in_page);
    Ok(())
}

/// Typed memory mapped into `p`: fresh contiguous RAM from the pool if it
/// was opened to allocate, else the pool's physical memory at the offset.
fn mem_map_tymem(rcvid: u64, p: &Process, msg: &[u8], ty: &super::tymem::Open) -> Result<(), u32> {
    let (addr, len) = (u64_at(msg, 8), u64_at(msg, 16));
    let (prot, flags) = (u32_at(msg, 24), u32_at(msg, 28));
    let size = span(len, 0).ok_or(ENOMEM)?;
    let at = addr - addr % PAGE_SIZE;
    let start = if ty.tflag & (super::tymem::ALLOCATE | super::tymem::ALLOCATE_CONTIG) != 0 {
        let pa = crate::vm::pmem::alloc_contig(size / PAGE_SIZE, ty.lo, ty.hi).ok_or(ENOMEM)?;
        map::map_contig(p, at, size, prot, flags, pa)?
    } else {
        let paddr = ty.lo.checked_add(u64_at(msg, 48)).ok_or(EINVAL)?;
        if paddr.checked_add(size - 1).is_none_or(|end| end > ty.hi) {
            return Err(EINVAL);
        }
        map::map_phys(p, at, size, prot, flags, paddr & !(PAGE_SIZE - 1))?
    };
    reply_mapped(rcvid, size, start, start);
    Ok(())
}

/// `_MEM_OFFSET` { type, subtype, reserved, addr, len }, subtype
/// `_MEM_OFFSET_PHYS` (`mem_offset()`): the physical address of `addr` in
/// the sender and how much of `len` is physically contiguous from there;
/// the reply is { size, offset, fd (-1) }; EACCES where nothing is
/// mapped.
fn mem_offset(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < 24 {
        return Err(EBADMSG);
    }
    // Every subtype is the physical address (`posix_mem_offset()` sends
    // `_MEM_OFFSET_FD` with object NOFD); FD and PT answer the same
    // whatever the descriptor (measured on procnto).
    if subtype > OFFSET_PT {
        return Err(ENOSYS);
    }
    let p = target(sender, 0).ok_or(ESRCH)?;
    let (addr, len) = (u64_at(msg, 8), u64_at(msg, 16));
    let page = addr & !(PAGE_SIZE - 1);
    // Nothing mapped there: EACCES (procnto), whatever the subtype.
    let (pa, _) = p.aspace().page(page).ok_or(crate::ker::errno::EACCES)?;
    let offset = pa + (addr - page);
    // Contiguous while the next page's frame follows.
    let mut size = PAGE_SIZE - (addr - page);
    while size < len {
        match p.aspace().page(page + size + (addr - page)) {
            Some((next, _)) if next == pa + size + (addr - page) => size += PAGE_SIZE,
            _ => break,
        }
    }
    let mut r = [0u8; 24];
    r[0..8].copy_from_slice(&size.min(len.max(1)).to_le_bytes());
    r[8..16].copy_from_slice(&offset.to_le_bytes());
    r[16..20].copy_from_slice(&(-1i32).to_le_bytes());
    reply_data(rcvid, 0, &r);
    Ok(())
}

// ANCHOR: mem_map_shm
/// A shared memory object mapped into `p`: its own pages with
/// `MAP_SHARED`, a copy of them with `MAP_PRIVATE`.
/// `revocable`: the object is (`SHMCTL_REVOCABLE`), so is the mapping.
fn mem_map_shm(rcvid: u64, p: &Process, msg: &[u8], obj: Arc<MemObject>, revocable: bool, max_prot: u32) -> Result<(), u32> {
    let (addr, len) = (u64_at(msg, 8), u64_at(msg, 16));
    let (prot, flags, align) = (u32_at(msg, 24), u32_at(msg, 28), u64_at(msg, 40));
    let offset = u64_at(msg, 48);
    offset_end(offset, len)?;
    let pg_off = offset % PAGE_SIZE;
    // As for files (`mem_map`).
    let in_page = if flags & map::MAP_FIXED != 0 { addr % PAGE_SIZE } else { pg_off };
    let size = span(len, in_page).ok_or(ENOMEM)?;
    let at = addr - addr % PAGE_SIZE;
    let name = Some(map::object_name("dev/shmem"));
    let start = if flags & MAP_TYPE == MAP_SHARED {
        map::map_shared(p, at, size, prot, flags, obj, offset / PAGE_SIZE, name, revocable, max_prot)?
    } else {
        map::map_copy(p, at, size, prot, flags, align, &obj, (offset - pg_off) / PAGE_SIZE, name)?
    };
    reply_mapped(rcvid, size, start, start + in_page);
    Ok(())
}
// ANCHOR_END: mem_map_shm

/// The physical memory at `paddr` mapped into `p`.
fn mem_map_phys(
    rcvid: u64,
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    paddr: u64,
) -> Result<(), u32> {
    // PROCMGR_AID_MEM_PHYS, for the physical address (as procnto).
    if !crate::ker::ability::allowed(p, crate::ker::ability::MEM_PHYS, Some(paddr)) {
        return Err(crate::ker::errno::EPERM);
    }
    let pg_off = paddr % PAGE_SIZE;
    if flags & map::MAP_FIXED != 0 && addr % PAGE_SIZE != pg_off {
        return Err(EINVAL);
    }
    // No length, or physical memory running past the top of the address
    // range or past the last physical address: no memory there (EINVAL,
    // as procnto, measured).
    let last = len.checked_sub(1).and_then(|l| paddr.checked_add(l)).ok_or(EINVAL)?;
    if last > crate::vm::pmem::phys_end() {
        return Err(EINVAL);
    }
    let size = span(len, pg_off).ok_or(EINVAL)?;
    let start = map::map_phys(
        p,
        addr - addr % PAGE_SIZE,
        size,
        prot,
        flags,
        paddr - pg_off,
    )?;
    reply_mapped(rcvid, size, start, start + pg_off);
    Ok(())
}

/// Reply to `_MEM_MAP`: the size, the mapping's start and the address
/// asked for.
fn reply_mapped(rcvid: u64, size: u64, start: u64, addr: u64) {
    let mut reply = [0u8; 24];
    reply[0..8].copy_from_slice(&size.to_le_bytes());
    reply[8..16].copy_from_slice(&start.to_le_bytes());
    reply[16..24].copy_from_slice(&addr.to_le_bytes());
    let _ = call(
        __KER_MSG_REPLYV,
        [rcvid, 0, reply.as_ptr() as u64, (-24i64) as u64, 0, 0],
    );
}

/// `_MEM_PEER` (`mmap_peer()`, `munmap_peer()`): `struct _mem_peer`
/// { type, peer_msg_len, pid, reserved }, then a `_MEM_MAP` or `_MEM_CTRL`
/// message about process `pid` (whose memory it maps; a file descriptor
/// is still the sender's). Takes `PROCMGR_AID_MEM_PEER` (as procnto).
fn peer(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    let pid = msg.get(4..8).map(|b| i32::from_le_bytes(b.try_into().unwrap())).ok_or(EINVAL)?;
    let inner = msg.get(16..).filter(|m| m.len() >= 4).ok_or(EBADMSG)?;
    let me = process::lookup(sender).ok_or(ESRCH)?;
    process::lookup(pid).ok_or(ESRCH)?;
    if !crate::ker::ability::allowed(&me, crate::ker::ability::MEM_PEER, Some(pid as u64)) {
        return Err(EPERM);
    }
    let ty = u16::from_le_bytes([inner[0], inner[1]]);
    match ty {
        MEM_MAP if inner.len() >= MEM_MAP_SIZE => {
            let mut m = inner.to_vec();
            m[4..8].copy_from_slice(&pid.to_le_bytes());
            mem_map(rcvid, sender, &m)
        }
        MEM_CTRL => {
            let subtype = u16::from_le_bytes([inner[2], inner[3]]);
            mem_ctrl(sender, pid, subtype, inner)?;
            let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
            Ok(())
        }
        _ => Err(EINVAL),
    }
}

fn mem_ctrl(sender: i32, pid: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < 24 {
        return Err(EBADMSG);
    }
    let flags = u32_at(msg, 4);
    let addr = u64_at(msg, 8);
    // At most up to the end of the address space.
    let len = match addr {
        0 => u64_at(msg, 16),
        _ => u64_at(msg, 16).min(0u64.wrapping_sub(addr)),
    };
    let p = target(sender, pid).ok_or(ESRCH)?;
    // The pages the range touches (procnto rounds it out); a range past
    // the end of the address arithmetic is unmap's EINVAL or protect's,
    // sync's and lock's ENOMEM (`map`, measured).
    let (page, span) = (addr & !(PAGE_SIZE - 1), len.saturating_add(addr % PAGE_SIZE));
    match subtype {
        // As procnto: a range touching no page (page-aligned, length 0)
        // isn't mapped.
        CTRL_PROTECT | CTRL_SYNC | CTRL_LOCK if span == 0 => Err(ENOMEM),
        // munmap_flags(): the low four bits (UNMAP_INIT_REQUIRED, _OPTIONAL
        // and two more) are accepted, anything above is EINVAL (procnto,
        // measured with each bit).
        CTRL_UNMAP if len != 0 && flags <= UNMAP_FLAGS => map::unmap(&p, page, span),
        CTRL_PROTECT
            if flags & map::PROT_WRITE != 0
                && flags & map::PROT_EXEC != 0
                && !crate::ker::ability::allowed(&p, crate::ker::ability::PROT_WRITE_AND_EXEC, Some(addr)) =>
        {
            Err(EPERM)
        }
        CTRL_PROTECT if flags & !PROT_MASK == 0 => map::protect(&p, page, span, flags),
        CTRL_UNMAP | CTRL_PROTECT => Err(EINVAL),
        // At most one of MS_SYNC, MS_ASYNC and MS_SYNC_FULL (MS_CACHE_ONLY
        // goes with any), and no unknown flags (procnto, measured).
        CTRL_SYNC => {
            let kind = flags & (MS_SYNC | MS_ASYNC | MS_SYNC_FULL);
            if kind & kind.wrapping_sub(1) != 0 || flags & !MS_KNOWN != 0 {
                return Err(EINVAL);
            }
            sync(&p, addr, len, flags)
        }
        // lock, unlock, advise: memory is always resident. Locking
        // (mlockall(), mlock()) takes MEM_LOCK; mlockall()'s flags are 1
        // to 3, mlock()'s range must be mapped (ENOMEM), the advice 0 to 5.
        CTRL_LOCKALL if !(1..=3).contains(&flags) => Err(EINVAL),
        CTRL_LOCKALL | CTRL_LOCK if !crate::ker::ability::allowed(&p, crate::ker::ability::MEM_LOCK, Some(addr)) => Err(EPERM),
        CTRL_LOCK if !mapped(&p, page, span) => Err(ENOMEM),
        CTRL_ADVISE if flags > 5 => Err(EINVAL),
        3..=CTRL_ADVISE => Ok(()),
        _ => Err(ENOSYS),
    }
}

/// Whether every page from `addr` for `len` bytes is mapped in `p`.
fn mapped(p: &Process, addr: u64, len: u64) -> bool {
    let end = addr.saturating_add(len);
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let mut at = addr;
    for r in regions.iter().filter(|r| r.end > addr && r.start < end) {
        if r.start > at {
            return false;
        }
        at = at.max(r.end);
    }
    at >= end
}

// msync() flags (<sys/mman.h>)
const MS_ASYNC: u32 = 0x1;
const MS_SYNC: u32 = 0x2;
/// `MS_SYNC_FULL`: a full sync (to the physical medium).
const MS_SYNC_FULL: u32 = 0x0800_0000;
/// The `msync()` flags procnto takes: MS_ASYNC, MS_SYNC, MS_INVALIDATE,
/// MS_INVALIDATE_LOCKED (0x0010_0000), MS_INVALIDATE_ICACHE, MS_CACHE_ONLY
/// (0x0200_0000) and MS_SYNC_FULL.
const MS_KNOWN: u32 = 0x0b10_0007;
const MS_INVALIDATE: u32 = 0x4;
const MS_INVALIDATE_ICACHE: u32 = 0x0100_0000;

/// `msync(addr, len, flags)` (`_MEM_CTRL_SYNC`): memory has no backing
/// store to write here, so it is the CPU caches (what drivers ask with
/// `MS_CACHE_ONLY` around a device's DMA): MS_SYNC/MS_ASYNC write the
/// data back, MS_INVALIDATE also drops it from the cache (the next read
/// comes from memory), MS_INVALIDATE_ICACHE makes it visible to
/// instruction fetch. Each page through the kernel's alias of its frame
/// (the data cache is physically indexed). Also `CacheControl()`.
pub fn sync(p: &Process, addr: u64, len: u64, flags: u32) -> Result<(), u32> {
    // A range past the end of the address space isn't mapped (ENOMEM).
    let end = addr.checked_add(len).ok_or(ENOMEM)?;
    let mut va = addr;
    while va < end {
        let page = va & !(PAGE_SIZE - 1);
        let off = va - page;
        let n = (PAGE_SIZE - off).min(end - va);
        let (pa, _) = p.aspace().page(page).ok_or(ENOMEM)?;
        crate::vm::kmem::with_frame(pa, |f| {
            let at = f.as_ptr() as u64 + off;
            if flags & MS_INVALIDATE != 0 {
                crate::arch::mmu::clean_inval_dcache(at, n);
            } else if flags & (MS_SYNC | MS_ASYNC) != 0 {
                crate::arch::mmu::clean_dcache(at, n);
            }
            if flags & MS_INVALIDATE_ICACHE != 0 {
                crate::arch::mmu::sync_icache(at, n);
            }
        });
        va += n;
    }
    Ok(())
}

fn reply_data(rcvid: u64, status: usize, data: &[u8]) {
    let _ = call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            status as u64,
            data.as_ptr() as u64,
            (-(data.len() as i64)) as u64,
            0,
            0,
        ],
    );
}

/// The process of an introspection request (`pid64_t`, 0: the sender).
fn introspected(sender: i32, msg: &[u8]) -> Result<crate::ker::process::ProcessRef, u32> {
    if msg.len() < 32 {
        return Err(EBADMSG);
    }
    let pid = u64_at(msg, 8);
    // The plain pid, or its 64-bit form exactly.
    if pid >> 32 != 0 && pid != super::debug::pid64(pid as u32 as i32) {
        return Err(ESRCH);
    }
    target(sender, pid as u32 as i32).ok_or(ESRCH)
}

/// The inode of the object a region maps: its file's, `/dev/zero`'s for
/// anonymous memory, none for physical memory.
fn object_ino(r: &map::RegionInfo) -> u64 {
    match r.object {
        Some(path) => imagefs::get()
            .and_then(|fs| fs.follow(path))
            .map_or(0, |(e, _)| e.ino as u64),
        None if r.flags & MAP_PHYS != 0 => 0,
        None => resmgr::DEV_ZERO_INO,
    }
}

/// The `struct _procfs_map_info`s of the regions of `p` from `start` to
/// `end`.
pub fn map_entries(p: &Process, start: u64, end: u64) -> Vec<u8> {
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let mut out = Vec::new();
    for r in regions.iter().filter(|r| r.end > start && r.start <= end) {
        out.extend_from_slice(&map_entry(r, r.start, r.end - r.start, 0));
    }
    out
}

/// A `procfs_mapinfo` of `size` bytes from `vaddr` of region `r`, at
/// physical address `paddr` (0: not reported).
fn map_entry(r: &map::RegionInfo, vaddr: u64, size: u64, paddr: u64) -> [u8; PROCFS_MAP_INFO_SIZE] {
    let mut e = [0u8; PROCFS_MAP_INFO_SIZE];
    let (flags, dev, ino, offset) = describe(r, vaddr);
    e[0..8].copy_from_slice(&vaddr.to_le_bytes());
    e[8..16].copy_from_slice(&size.to_le_bytes());
    e[16..20].copy_from_slice(&flags.to_le_bytes());
    e[20..24].copy_from_slice(&dev.to_le_bytes());
    e[24..32].copy_from_slice(&offset.to_le_bytes());
    e[32..40].copy_from_slice(&ino.to_le_bytes());
    e[40..48].copy_from_slice(&paddr.to_le_bytes());
    e
}

/// A region as procnto describes it from `vaddr` on (measured): its flags
/// (those it was mapped with but `MAP_LAZY`, its protection, and what
/// backs it), device, inode and offset. No `PG_*` bits.
/// - Anonymous memory (or private `/dev/zero`): `MAP_SYSRAM | MAP_ANON`,
///   `/dev/zero`'s inode on device 1.
/// - Shared anonymous or physically contiguous memory: also `MAP_PHYS`,
///   and shared.
/// - A file's data in the image: `MAP_PHYS` without `MAP_SYSRAM`, on the
///   image's device, at the data's physical address.
/// - A device's memory: `MAP_PHYS`, `/dev/mem`'s device, at its address.
/// - A shared memory object, shared or private: `MAP_SYSRAM`, device 1.
fn describe(r: &map::RegionInfo, vaddr: u64) -> (u32, u32, u64, u64) {
    let base = (r.flags & !MAP_LAZY) | r.prot;
    let anon = r.flags & MAP_ANON != 0 || r.object == Some("dev/zero");
    let at = |pa: u64| pa + (vaddr - r.start);
    match r.phys {
        Some(pa) if r.image => (base | MAP_PHYS, IFS_DEV, object_ino(r), at(pa)),
        Some(pa) if !anon => (base | MAP_PHYS, PHYS_DEV, 1, at(pa)),
        _ if anon && (r.flags & MAP_PHYS != 0 || r.shared) => {
            ((base & !MAP_TYPE) | MAP_SHARED | MAP_PHYS | MAP_SYSRAM | MAP_ANON, ANON_DEV, 0, 0)
        }
        // The main thread's stack's guard page has no memory.
        _ if anon && r.prot == 0 && r.flags & MAIN_STACK == MAIN_STACK => (base, ANON_DEV, resmgr::DEV_ZERO_INO, 0),
        _ if anon => (base | MAP_SYSRAM | MAP_ANON, ANON_DEV, resmgr::DEV_ZERO_INO, 0),
        // A file of the image (copied, a writable private mapping).
        _ if object_ino(r) != 0 => (base | MAP_PHYS, IFS_DEV, object_ino(r), 0),
        _ => (base | MAP_SYSRAM, ANON_DEV, 0, 0),
    }
}

/// A guard page below a stack (`pmap` shows it as the stack's guard size).
fn is_guard(r: &map::RegionInfo) -> bool {
    r.prot == 0 && r.flags & map::MAP_STACK != 0
}

/// A file's pages (of the image, mapped in place or copied).
fn is_file(r: &map::RegionInfo) -> bool {
    r.image || r.object.is_some_and(|o| o != "dev/zero") && object_ino(r) != 0
}

/// What `pmap` shows of a region: its flags as mapped (but `MAP_LAZY` and
/// the main stack's mark; `MAP_PHYS` for a file's pages), maximum
/// protection, reserved bytes, path and kind of memory.
struct PmapRegion {
    flags: u32,
    maxprot: u32,
    rsv: u64,
    path: String,
    asinfo: &'static str,
}

fn pmap_region(r: &map::RegionInfo) -> PmapRegion {
    let shared = r.flags & MAP_TYPE == MAP_SHARED;
    let anon = r.flags & MAP_ANON != 0 || r.object == Some("dev/zero");
    let mut flags = r.flags & !(MAP_LAZY | 0x8000_0000);
    if is_file(r) {
        flags |= MAP_PHYS;
    }
    let (path, asinfo) = if is_file(r) {
        (alloc::format!("/{}", r.object.unwrap_or("")), "{ram}")
    } else if anon && shared {
        ("**anonymous**".into(), "{unknown}")
    } else if anon && r.flags & map::MAP_STACK != 0 {
        ("{stack}".into(), "{sysram}")
    } else if anon && r.flags & map::MAP_ELF != 0 {
        ("{bss}".into(), "{sysram}")
    } else if anon {
        ("{heap}".into(), "{sysram}")
    } else if let Some(o) = r.object {
        (alloc::format!("/{o}"), "{sysram}")
    } else {
        ("{phys}".into(), "{unknown}")
    };
    PmapRegion {
        flags,
        maxprot: if is_file(r) && shared { 0x0d } else { 0x0f },
        rsv: if shared || r.phys.is_some() && !r.image { 0 } else { r.end - r.start },
        path,
        asinfo,
    }
}

/// `refcnt` and `mapcnt` of the object a region maps, approximated: an
/// object mapped shared, its references and its mappings; the image's
/// files (one object on procnto), the mappings of them all and two more
/// references; anonymous memory, no references and all its mappings.
fn object_counts(r: &map::RegionInfo, all: &[map::RegionInfo]) -> (u64, u64) {
    let anon = |q: &map::RegionInfo| q.flags & MAP_ANON != 0 && !q.shared;
    if r.shared_ref.0 != 0 {
        let maps = all.iter().filter(|q| q.shared_ref.0 == r.shared_ref.0).count() as u64;
        (r.shared_ref.1 as u64, maps)
    } else if is_file(r) {
        let maps = all.iter().filter(|q| is_file(q)).count() as u64;
        (maps + 2, maps)
    } else if anon(r) {
        (0, all.iter().filter(|q| anon(q)).count() as u64)
    } else {
        (0, 0)
    }
}

fn every_region() -> Vec<map::RegionInfo> {
    process::all().iter().flat_map(|p| p.mem.lock().regions().collect::<Vec<_>>()).collect()
}

/// `/proc/<pid>/pmap` (procnto's format, measured): a line per region
/// (guard pages folded into their stacks) after the header.
pub fn pmap(p: &Process) -> Vec<u8> {
    const HEADER: &str = "vaddr,size,flags,prot,maxprot,dev,ino,offset,rsv,guardsize,refcnt,mapcnt,path,asinfo\n";
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let all = every_region();
    let mut text = String::from(HEADER);
    for (i, r) in regions.iter().enumerate() {
        if is_guard(r) {
            continue;
        }
        let guard = match i.checked_sub(1).map(|j| &regions[j]) {
            Some(g) if r.flags & map::MAP_STACK != 0 && is_guard(g) && g.end == r.start => g.end - g.start,
            _ => 0,
        };
        let m = pmap_region(r);
        let (_, dev, ino, _) = describe(r, r.start);
        let (refcnt, mapcnt) = object_counts(r, &all);
        let offset = if is_file(r) { r.offset } else { 0 };
        text += &alloc::format!(
            "0x{:016x},0x{:016x},0x{:08x},0x{:02x},0x{:02x},0x{:08x},0x{:016x},0x{:016x},0x{:016x},0x{:08x},0x{:08x},0x{:08x},{},{}\n",
            r.start,
            r.end - r.start,
            m.flags,
            r.prot >> 8 & 0xf,
            m.maxprot,
            dev,
            ino,
            offset,
            m.rsv,
            guard,
            refcnt,
            mapcnt,
            m.path,
            m.asinfo
        );
    }
    text.into_bytes()
}

/// `/proc/<pid>/mappings` (procnto's format, measured): a line per page
/// mapped, with its physical address.
pub fn mappings(p: &Process) -> Vec<u8> {
    const HEADER: &str = "mi_vaddr,ri_flags,mi_flags,mi_pte_prot,mi_prot,ri_prot,ri_maxprot,mi_paddr,mi_hwattr,mi_pgsize,mi_sharecnt,dev,ino,ri_offset,object_name\n";
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let aspace = p.aspace();
    let mut text = String::from(HEADER);
    for r in regions.iter().filter(|r| !is_guard(r)) {
        let m = pmap_region(r);
        let (_, dev, ino, _) = describe(r, r.start);
        let prot = r.prot >> 8 & 0xf;
        for va in (r.start..r.end).step_by(PAGE_SIZE as usize) {
            let pa = aspace.page(va).map_or(0, |(pa, _)| pa);
            text += &alloc::format!(
                "0x{:x},0x{:08x},0x00000014,0x{:08x},0x{:08x},0x{:02x},0x{:02x},0x{:016x},0x00000000,0x{:08x},00000000,0x{:08x},0x{:016x},0x{:016x},{}\n",
                va,
                m.flags,
                prot,
                prot,
                prot,
                m.maxprot,
                pa,
                PAGE_SIZE,
                dev,
                ino,
                r.offset + (va - r.start),
                m.path
            );
        }
    }
    text.into_bytes()
}

/// A count of pages as procnto prints it: hexadecimal, then the size in
/// kB, MB or GB with three decimals (cut, not rounded).
pub fn pages_size(pages: u64) -> String {
    let bytes = pages * PAGE_SIZE;
    let (unit, div) = if bytes < 1 << 20 {
        ("kB", 1u64 << 10)
    } else if bytes < 1 << 30 {
        ("MB", 1 << 20)
    } else {
        ("GB", 1 << 30)
    };
    let thousandths = bytes as u128 * 1000 / div as u128;
    alloc::format!("0x{:x} ({}.{:03}{})", pages, thousandths / 1000, thousandths % 1000, unit)
}

/// `/proc/<pid>/vmstat`: the address space's counts, in pages: regions;
/// 64 KB runs of memory (`vm_map`, approximated: per region, the image's
/// pages mapped in place left out); private and shared
/// pages (guard pages left out) and their sum; the image's shared pages;
/// the private anonymous memory that isn't a stack or a `.bss` (the
/// heap, `rlimit_data`).
pub fn vmstat(p: &Process) -> Vec<u8> {
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let pages = |r: &map::RegionInfo| (r.end - r.start) / PAGE_SIZE;
    let shared = |r: &map::RegionInfo| r.flags & MAP_TYPE == MAP_SHARED;
    let held: Vec<&map::RegionInfo> = regions.iter().filter(|r| !is_guard(r)).collect();
    let private: u64 = held.iter().filter(|r| !shared(r)).map(|r| pages(r)).sum();
    let shared_pages: u64 = held.iter().filter(|r| shared(r)).map(|r| pages(r)).sum();
    let phys: u64 = held.iter().filter(|r| shared(r) && r.phys.is_some()).map(|r| pages(r)).sum();
    let heap: u64 = held
        .iter()
        .filter(|r| !shared(r) && r.flags & MAP_ANON != 0 && r.flags & (map::MAP_ELF | map::MAP_STACK) == 0)
        .map(|r| pages(r))
        .sum();
    let runs: u64 = held.iter().filter(|r| !r.image).map(|r| pages(r).div_ceil(16)).sum();
    let total = private + shared_pages;
    let mut text = alloc::format!(
        "as_stats.vm_region={}\nas_stats.vm_map={}\n",
        regions.len(),
        runs
    );
    for (name, v) in [
        ("map_size", total),
        ("map_phys", phys),
        ("map_shared", shared_pages),
        ("map_private", private),
        ("rss", total),
        ("anon_rsv", private),
        ("rlimit_data", heap),
    ] {
        text += &alloc::format!("as_stats.{}={}\n", name, pages_size(v));
    }
    text.into_bytes()
}

/// `_MEM_MAP_INFO_PHYSICAL` (`per_page` false: the physically contiguous
/// runs of the regions from `start` to `end`, whole) or `_PAGETABLE`
/// (each page from `start` to `end`, both included), as procnto: with
/// their physical addresses.
fn phys_entries(p: &Process, start: u64, end: u64, per_page: bool) -> Vec<u8> {
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let aspace = p.aspace();
    let pa = |va: u64| aspace.page(va).map_or(0, |(pa, _)| pa);
    let mut out = Vec::new();
    for r in regions.iter().filter(|r| r.end > start && r.start <= end) {
        if per_page {
            let from = r.start.max(start & !(PAGE_SIZE - 1));
            let to = r.end.min(end.saturating_add(1));
            for va in (from..to).step_by(PAGE_SIZE as usize) {
                out.extend_from_slice(&map_entry(r, va, PAGE_SIZE, pa(va)));
            }
            continue;
        }
        let mut run = r.start;
        while run < r.end {
            let base = pa(run);
            let mut next = run + PAGE_SIZE;
            while next < r.end && base != 0 && pa(next) == base + (next - run) {
                next += PAGE_SIZE;
            }
            if next > start && run <= end {
                out.extend_from_slice(&map_entry(r, run, next - run, base));
            }
            run = next;
        }
    }
    out
}

/// `_MEM_AS_INFO` { type, subtype, pid }: process `pid`'s (0: the
/// sender's) address space as `struct _debug_aspace`. Memory is always
/// resident (superlocked), so the resident set is all that is mapped.
fn as_info(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    let pid = u32_at(msg, 4) as i32;
    let p = target(sender, pid).ok_or(ESRCH)?;
    let (used, count, private) = {
        let m = p.mem.lock();
        let (mut used, mut count) = (0u64, 0u64);
        for r in m.regions() {
            used += r.end - r.start;
            count += 1;
        }
        (used, count, m.private_bytes())
    };
    let size = if p.pid == process::KERNEL_PID { u64::MAX } else { crate::arch::uspace::USER_END - 1 };
    let mut r = [0u8; DEBUG_ASPACE_SIZE];
    for (i, v) in [size, used, count, private, used - private, used].iter().enumerate() {
        r[8 * i..8 * i + 8].copy_from_slice(&v.to_le_bytes());
    }
    reply_data(rcvid, 0, &r);
    Ok(())
}

/// The `procfs_debuginfo` of the object mapped at `addr` in `p`: its
/// lowest address and path (`/dev/zero` for anonymous memory, none for
/// physical memory). Where nothing is mapped, the next region above
/// (as procnto); ENXIO if there is none.
pub fn object_at(p: &Process, addr: u64) -> Result<Vec<u8>, u32> {
    let regions: Vec<map::RegionInfo> = p.mem.lock().regions().collect();
    let r = regions.iter().find(|r| addr < r.end).ok_or(ENXIO)?;
    let (base, path) = match r.object {
        Some(name) => {
            let base = regions
                .iter()
                .filter(|q| q.object == Some(name))
                .map(|q| q.start)
                .min()
                .unwrap_or(r.start);
            (base, alloc::format!("/{name}"))
        }
        None if r.flags & MAP_PHYS != 0 => (r.start, alloc::string::String::new()),
        None => (r.start, "/dev/zero".into()),
    };
    let mut out = base.to_le_bytes().to_vec();
    out.extend_from_slice(path.as_bytes());
    out.push(0);
    Ok(out)
}

fn map_info(rcvid: u64, sender: i32, subtype: u16, msg: &[u8]) -> Result<(), u32> {
    let p = introspected(sender, msg)?;
    let (start, end) = (u64_at(msg, 16), u64_at(msg, 24));
    let out = match subtype {
        MAP_INFO_VIRTUAL if start > end => return Err(EINVAL),
        MAP_INFO_VIRTUAL => map_entries(&p, start, end),
        MAP_INFO_PHYSICAL | MAP_INFO_PAGETABLE => phys_entries(&p, start, end, subtype == MAP_INFO_PAGETABLE),
        MAP_INFO_OBJECT => object_at(&p, start)?,
        _ => return Err(ENOSYS),
    };
    reply_data(rcvid, out.len(), &out);
    Ok(())
}

fn read_mem(rcvid: u64, sender: i32, msg: &[u8], room: usize) -> Result<(), u32> {
    let p = introspected(sender, msg)?;
    let addr = u64_at(msg, 16);
    let end = p
        .mem
        .lock()
        .regions()
        .find(|r| r.start <= addr && addr < r.end)
        .map(|r| r.end)
        .ok_or(ESRVRFAULT)?;
    let n = room.min((end - addr) as usize).min(64 * 1024);
    let mut buf = alloc::vec![0u8; n];
    // SAFETY: `p` is held meanwhile.
    umem::read(unsafe { umem::Space::user(&p) }, addr, &mut buf).map_err(|_| ESRVRFAULT)?;
    reply_data(rcvid, n, &buf);
    Ok(())
}

fn mem_info(rcvid: u64, sender: i32, msg: &[u8]) -> Result<(), u32> {
    if msg.len() < 16 {
        return Err(EBADMSG);
    }
    let fd = i32::from_le_bytes(msg[4..8].try_into().unwrap());
    let (free, total) = if fd == -1 {
        (
            crate::vm::pmem::free_pages() as u64 * PAGE_SIZE,
            crate::vm::pmem::total_bytes(),
        )
    } else {
        // A typed memory object: the most it can allocate, as it was
        // opened to allocate (procnto, measured, review PATHMEM-26):
        // scattered, the pool's free memory; contiguous, its largest free
        // run; not to allocate, nothing. And its size.
        let ty = resmgr::tymem_object(resmgr::Client {
            pid: sender,
            coid: fd,
        })
        .ok_or(ENODEV)?;
        let free = if ty.tflag & super::tymem::ALLOCATE_CONTIG != 0 {
            crate::vm::pmem::largest_contig(ty.lo, ty.hi)
        } else if ty.tflag & super::tymem::ALLOCATE != 0 {
            crate::vm::pmem::free_in(ty.lo, ty.hi)
        } else {
            0
        };
        (free, ty.hi - ty.lo + 1)
    };
    let mut r = [0u8; MEM_INFO_REPLY_SIZE];
    r[16..24].copy_from_slice(&free.to_le_bytes());
    r[24..32].copy_from_slice(&total.to_le_bytes());
    reply_data(rcvid, 0, &r);
    Ok(())
}

/// Handle a `_MEM_*` message from process `sender`, whose reply buffer
/// holds `room` bytes; replies to it.
pub fn handle(rcvid: u64, sender: i32, ty: u16, subtype: u16, msg: &[u8], room: usize) {
    let r = match ty {
        MEM_MAP => mem_map(rcvid, sender, msg),
        MEM_PEER => peer(rcvid, sender, msg),
        MEM_INFO => mem_info(rcvid, sender, msg),
        MEM_OFFSET => mem_offset(rcvid, sender, subtype, msg),
        MEM_MAP_INFO => map_info(rcvid, sender, subtype, msg),
        MEM_READ_MEM => read_mem(rcvid, sender, msg, room),
        MEM_CTRL => mem_ctrl(sender, 0, subtype, msg).map(|()| {
            let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
        }),
        MEM_AS_INFO => as_info(rcvid, sender, msg),
        // The loader's notes of what it mapped (for debuggers): taken, as
        // procnto, and not kept.
        MEM_DEBUG_INFO | MEM_DEBUG_INFO64 => {
            let _ = call(__KER_MSG_REPLYV, [rcvid, 0, 0, 0, 0, 0]);
            Ok(())
        }
        // Adding physical memory isn't supported: procnto refuses memory
        // it can't use the same way (EINVAL).
        MEM_PMEM_ADD => Err(EINVAL),
        // A memory object's segments: EBADF for every descriptor on
        // procnto, shared memory objects included (measured).
        MEM_OBJ_INFO => Err(crate::ker::errno::EBADF),
        _ => Err(ENOSYS),
    };
    if let Err(e) = r {
        error(rcvid, e);
    }
    super::procevent::values_changed();
}

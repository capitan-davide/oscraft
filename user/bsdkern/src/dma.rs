//! Memory a device reaches: DMA buffers and device registers, through the
//! process manager's `_MEM_MAP` and `_MEM_OFFSET` (a buffer's physical
//! address), as QNX's drivers get theirs. FreeBSD's busdma
//! (`kern/busdma.c`) and bus space sit on these.
//!
//! A DMA buffer comes from a typed-memory pool, the system page's named
//! ranges of RAM (`below1G`, `below4G`): the largest pool all of which the
//! device reaches (the busdma tag's `lowaddr`), or the one the server was
//! told (`hw.bsdkern.dma_pool`, as `io-usb-otg -t memory=`). Measured with
//! REA, io-usb-otg's `alloc_resource_typed_memory()` does the same:
//! `posix_typed_mem_open()` of the pool (`/memory/below4G` by default),
//! `mmap()` of it shared, `mem_offset()` of the buffer. Without a pool
//! that fits, contiguous RAM below 4 GB (`MAP_PHYS | MAP_ANON`).

use core::ffi::c_void;
use core::ptr::null_mut;

use kercall::abi::{Iov, SYSMGR_COID};
use kercall::neutrino::msg::MsgSendv_r;
use proto::mem::{CTRL_UNMAP, Ctrl, MEM_CTRL, MEM_MAP, MEM_OFFSET, Map, MapReply, OFFSET_PHYS, Offset, OffsetReply};

const PROT_READ_WRITE: u32 = 0x100 | 0x200;
const PROT_NOCACHE: u32 = 0x800;
const MAP_PRIVATE: u32 = 0x2;
const MAP_SHARED: u32 = 0x1;
const MAP_PHYS: u32 = 0x10000;
const MAP_ANON: u32 = 0x80000;
const NOFD: i32 = -1;

fn ask<Q, A: Default>(q: &Q) -> Option<A> {
    let mut a = A::default();
    let send = [Iov::raw((q as *const Q).cast(), size_of::<Q>())];
    let recv = [Iov::raw((&raw mut a).cast(), size_of::<A>())];
    // SAFETY: both are on this stack, of their sizes.
    let r = unsafe { MsgSendv_r(SYSMGR_COID, send.as_ptr().cast(), 1, recv.as_ptr().cast(), 1) };
    (r >= 0).then_some(a)
}

fn map(len: usize, prot: u32, flags: u32, offset: u64) -> *mut c_void {
    map_fd(len, prot, flags, NOFD, offset)
}

fn map_fd(len: usize, prot: u32, flags: u32, fd: i32, offset: u64) -> *mut c_void {
    let m = Map { r#type: MEM_MAP, len: len as u64, prot, flags, fd, offset: offset as i64, ..Map::default() };
    ask::<Map, MapReply>(&m).map_or(null_mut(), |r| r.addr as *mut c_void)
}

/// The pools, by how much of the address space they hold (`below<N>`:
/// all of it below N).
const POOLS: [(&str, u64); 2] = [("/memory/below1G", 1 << 30), ("/memory/below4G", 1 << 32)];
/// `posix_typed_mem_open()`'s `POSIX_TYPED_MEM_ALLOCATE_CONTIG`.
const ALLOCATE_CONTIG: u32 = 0x2;

/// The pools opened so far: their names and connections (-1: none there).
static OPEN: std::sync::Mutex<Vec<(String, i32)>> = std::sync::Mutex::new(Vec::new());

/// The connection to pool `name`, opened once as libc's
/// `posix_typed_mem_open(name, O_RDWR, POSIX_TYPED_MEM_ALLOCATE_CONTIG)`
/// does: a connect of `_FTYPE_TYMEM` with the flags as its extra data.
fn pool(name: &str) -> Option<i32> {
    let mut open = OPEN.lock().ok()?;
    if let Some(&(_, fd)) = open.iter().find(|(n, _)| n == name) {
        return (fd >= 0).then_some(fd);
    }
    let tflag = ALLOCATE_CONTIG.to_le_bytes();
    let req = kercall::path::Request {
        subtype: proto::connect::CONNECT_OPEN,
        file_type: proto::connect::FTYPE_TYMEM,
        ioflag: 2, // O_RDWR
        mode: 0,
        sflag: 0,
        access: 3, // _IO_FLAG_RD | _IO_FLAG_WR
        dirfd: None,
        extra_type: proto::connect::EXTRA_TYMEM,
        extra: &tflag,
        base: 0,
        cancellable: false,
        coflags: 0,
    };
    let path = std::ffi::CString::new(name).ok()?;
    let mut reply = [0u8; 64];
    // SAFETY: a string, and the extra data is the type's.
    let fd = unsafe { kercall::path::connect(&req, path.as_ptr(), &mut reply) }.map_or(-1, |c| c.coid);
    open.push((name.to_owned(), fd));
    if crate::env::is_verbose() {
        eprintln!("{}: DMA from {name}{}", crate::env::me(), if fd < 0 { ": not there" } else { "" });
    }
    (fd >= 0).then_some(fd)
}

// ANCHOR: dma_pool_for
/// The pool for a device that reaches addresses up to `lowaddr`: the one
/// the server was told (`hw.bsdkern.dma_pool`), else the largest pool all
/// below `lowaddr`.
fn pool_for(lowaddr: u64) -> Option<i32> {
    if let Some(name) = crate::env::kenv_get("hw.bsdkern.dma_pool") {
        let name = if name.starts_with('/') { name } else { format!("/memory/{name}") };
        return pool(&name);
    }
    POOLS.iter().rev().filter(|(_, size)| size - 1 <= lowaddr).find_map(|(name, _)| pool(name))
}
// ANCHOR_END: dma_pool_for

// ANCHOR: dma_alloc
/// `len` bytes of physically contiguous RAM a device reaching addresses up
/// to `lowaddr` can use, uncached if `nocache`, its physical address in
/// `*pa`; null if there is none.
///
/// # Safety
///
/// `pa` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_dma_alloc(len: usize, nocache: bool, lowaddr: u64, pa: *mut u64) -> *mut c_void {
    let prot = PROT_READ_WRITE | if nocache { PROT_NOCACHE } else { 0 };
    let va = match pool_for(lowaddr) {
        Some(fd) => map_fd(len, prot, MAP_SHARED, fd, 0),
        None => map(len, prot, MAP_PHYS | MAP_ANON | MAP_PRIVATE, 0),
    };
    if !va.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { pa.write(bsdkern_phys(va, null_mut())) };
    }
    va
}
// ANCHOR_END: dma_alloc

/// Gives back what `bsdkern_dma_alloc()` or `bsdkern_map_device()` gave.
///
/// # Safety
///
/// `va` and `len` are such a mapping, unused from now on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_unmap(va: *mut c_void, len: usize) {
    let c = Ctrl { r#type: MEM_CTRL, subtype: CTRL_UNMAP, flags: 0, addr: va as u64, len: len as u64 };
    let _ = ask::<Ctrl, [u64; 2]>(&c);
}

/// The physical address of `va`, 0 if it has none; in `*contig`, if not
/// null, how many bytes from there are physically contiguous.
///
/// # Safety
///
/// `contig` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bsdkern_phys(va: *const c_void, contig: *mut usize) -> u64 {
    let q = Offset { r#type: MEM_OFFSET, subtype: OFFSET_PHYS, reserved: 0, addr: va as u64, len: u64::MAX >> 1 };
    let Some(r) = ask::<Offset, OffsetReply>(&q) else { return 0 };
    if !contig.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { contig.write(r.size as usize) };
    }
    r.offset as u64
}

/// A device's registers at physical `pa`, `len` bytes, uncached.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_map_device(pa: u64, len: usize) -> *mut c_void {
    map(len, PROT_READ_WRITE | PROT_NOCACHE, MAP_PHYS | MAP_SHARED, pa)
}

/// Memory at physical `pa`, `len` bytes, read-only and cached: a table the
/// firmware left in RAM (the device tree), not a device.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_map_table(pa: u64, len: usize) -> *mut c_void {
    map(len, 0x100, MAP_PHYS | MAP_SHARED, pa)
}

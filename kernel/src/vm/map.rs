//! A process's user memory map (the regions QNX's memmgr tracks for
//! `mmap()`, `munmap()` and `mprotect()`, `<sys/mman.h>`).
//!
//! Each region is a page-aligned range with QNX `PROT_*` bits. Memory is
//! allocated (zeroed, or filled from a file) when it is mapped, not on
//! first touch.
//! Pages of a `PROT_NONE` region are only allocated once they are made
//! accessible; pages turned `PROT_NONE` keep their frame (EL1-only), so
//! their contents survive a later `mprotect()`.
//!
//! File mappings are private copies (the image filesystem's files are not
//! page-aligned, so they cannot be mapped in place). Unmapped frames go
//! back to `pmem`. Shared mappings of a memory object (`MAP_SHARED` of a
//! `/dev/shmem` object, see `vm::object`) map the object's own frames:
//! they stay the object's when unmapped, and a forked child shares them.
//! As QNX 8's memory is "superlocked", nothing is faulted in: a fault is an
//! error (`MAP_LAZY` is ignored). `fork()` copies the parent's private
//! pages at once; with procnto's OsCraft-only `-mc` option it shares them
//! copy-on-write instead (faster, but each first write faults).

use alloc::collections::BTreeMap;
use alloc::sync::Arc;

use crate::arch::mmu;
use crate::arch::uspace::{USER_END, UserProt};
use crate::ker::errno::{EINVAL, ENOMEM};
use crate::ker::process::Process;
use crate::vm::object::MemObject;
use crate::vm::pmem::PAGE_SIZE;
use crate::vm::{kmem, pmem};

// <sys/mman.h>
pub const PROT_READ: u32 = 0x100;
pub const PROT_WRITE: u32 = 0x200;
pub const PROT_EXEC: u32 = 0x400;
pub const MAP_SHARED: u32 = 0x1;
pub const MAP_PRIVATE: u32 = 0x2;
pub const MAP_FIXED: u32 = 0x10;
pub const MAP_ELF: u32 = 0x20;
/// Copy-on-write `fork()` (procnto `-mc`, an OsCraft option; off by
/// default, as QNX 8 copies at once).
static COW_FORK: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Choose copy-on-write `fork()` (at boot).
pub fn set_cow_fork(on: bool) {
    COW_FORK.store(on, core::sync::atomic::Ordering::Relaxed);
}

pub const MAP_STACK: u32 = 0x1000;
pub const PROT_NONE: u32 = 0;
pub const MAP_ANON: u32 = 0x8_0000;
/// Not copied into a child by fork.
pub const MAP_NOINHERIT: u32 = 0x2_0000;
/// Uncached (device) memory.
pub const PROT_NOCACHE: u32 = 0x800;

/// Where `mmap()` without an address starts looking for room (above the
/// program and its dynamic loader, see `proc::loader`).
const MMAP_BASE: u64 = 1 << 36;

#[derive(Clone)]
struct Region {
    end: u64,
    prot: u32,
    /// How it was mapped: `MAP_*` (`MAP_PRIVATE`, `MAP_ANON`, `MAP_ELF`
    /// for program segments, `MAP_STACK`...), for introspection.
    flags: u32,
    /// The file it maps (its path, without the leading '/'), if any.
    object: Option<&'static str>,
    /// Physical memory (a device's registers) mapped at `phys` onwards:
    /// not ours to free, shared rather than copied by fork.
    phys: Option<u64>,
    /// `phys` is a file's data in the image filesystem, mapped read-only
    /// and private: made the process's own copy before it is written.
    image: bool,
    /// A memory object mapped shared, from the given page of it on: its
    /// frames, not ours to free, shared by fork.
    shared: Option<(Arc<MemObject>, u64)>,
    /// Mapped while its object was revocable (`SHMCTL_REVOCABLE`): its
    /// creator's `shm_revoke()` takes it away, and fork doesn't copy it.
    revocable: bool,
    /// The offset in `object` of its first page, for introspection.
    offset: u64,
    /// The most `mprotect()` may give it: a shared mapping of a
    /// descriptor not open for writing can't become writable (EACCES,
    /// measured, PATHMEM-3); everything else may take any `PROT_*`.
    max_prot: u32,
}

/// `max_prot` of a region with no cap.
pub const PROT_ALL: u32 = PROT_READ | PROT_WRITE | PROT_EXEC;

/// A region of an address space, as introspection reports it.
pub struct RegionInfo {
    pub start: u64,
    pub end: u64,
    pub prot: u32,
    pub flags: u32,
    pub object: Option<&'static str>,
    /// Physical memory mapped (a device's, or a file's data in the image
    /// filesystem): its address at `start`.
    pub phys: Option<u64>,
    /// `phys` is a file's data in the image filesystem.
    pub image: bool,
    /// A memory object mapped shared (a shared memory object, or shared
    /// anonymous memory).
    pub shared: bool,
    /// The offset in `object` (a file) of `start`.
    pub offset: u64,
    /// The memory object mapped shared, as an identity and its number of
    /// references (0, 0 for none).
    pub shared_ref: (usize, usize),
}

/// Mapped objects' paths, kept for good (a path per file of the image
/// filesystem at most).
static OBJECT_NAMES: crate::sync::IrqSpinLock<alloc::vec::Vec<&'static str>> =
    crate::sync::IrqSpinLock::new(alloc::vec::Vec::new());

/// A `'static` copy of `path`.
pub fn object_name(path: &str) -> &'static str {
    let mut names = OBJECT_NAMES.lock();
    if let Some(n) = names.iter().find(|n| **n == path) {
        return n;
    }
    let n: &'static str = alloc::boxed::Box::leak(path.into());
    names.push(n);
    n
}

pub struct MemMap {
    /// Keyed by start address; regions never overlap.
    regions: BTreeMap<u64, Region>,
}

fn user_prot(prot: u32) -> UserProt {
    UserProt {
        read: prot & PROT_READ != 0,
        write: prot & PROT_WRITE != 0,
        exec: prot & PROT_EXEC != 0,
    }
}

fn page_round(len: u64) -> Option<u64> {
    len.checked_add(PAGE_SIZE - 1).map(|l| l & !(PAGE_SIZE - 1))
}

impl MemMap {
    /// The bytes of memory that are the process's own: not device memory
    /// nor a memory object it shares (`PROCMGR_VALUE_PRIVATE_MEM`); image
    /// pages count once it has its own copy.
    pub fn private_bytes(&self) -> u64 {
        self.regions
            .iter()
            .filter(|(_, r)| r.phys.is_none() && r.shared.is_none())
            .map(|(&s, r)| r.end - s)
            .sum()
    }

    pub const fn new() -> Self {
        Self {
            regions: BTreeMap::new(),
        }
    }

    /// Split the region containing `at` (if any) so that a region starts there.
    fn split(&mut self, at: u64) {
        let Some((&start, r)) = self.regions.range(..at).next_back() else {
            return;
        };
        if r.end > at {
            let r = r.clone();
            self.regions.insert(
                start,
                Region {
                    end: at,
                    ..r.clone()
                },
            );
            let phys = r.phys.map(|pa| pa + (at - start));
            let shared = r
                .shared
                .clone()
                .map(|(o, page)| (o, page + (at - start) / PAGE_SIZE));
            let offset = r.offset + (at - start);
            self.regions.insert(at, Region { phys, shared, offset, ..r });
        }
    }

    /// Record that the region at `start` maps its object from `offset` on
    /// (a file mapped at an offset).
    pub fn set_offset(&mut self, start: u64, offset: u64) {
        if let Some(r) = self.regions.get_mut(&start) {
            r.offset = offset;
        }
    }

    fn is_free(&self, start: u64, end: u64) -> bool {
        let before = self.regions.range(..end).next_back();
        before.is_none_or(|(_, r)| r.end <= start)
    }

    /// Lowest free range of `len` bytes at or above `from`, aligned to `align`.
    fn find_free(&self, from: u64, len: u64, align: u64) -> Option<u64> {
        let mut at = from.next_multiple_of(align);
        for (&start, r) in self.regions.range(..) {
            if r.end <= at {
                continue;
            }
            if start >= at.checked_add(len)? {
                break;
            }
            at = r.end.next_multiple_of(align);
        }
        (at.checked_add(len)? <= USER_END).then_some(at)
    }

    /// The regions, in address order.
    pub fn regions(&self) -> impl Iterator<Item = RegionInfo> + '_ {
        self.regions.iter().map(|(&start, r)| RegionInfo {
            start,
            end: r.end,
            prot: r.prot,
            flags: r.flags,
            object: r.object,
            phys: r.phys,
            image: r.image,
            shared: r.shared.is_some(),
            offset: r.offset,
            shared_ref: r
                .shared
                .as_ref()
                .map_or((0, 0), |(o, _)| (Arc::as_ptr(o) as usize, Arc::strong_count(o))),
        })
    }

    /// Record a range the loader mapped (program segments, stack). Pages
    /// already recorded (segments sharing a page) are left as they are:
    /// only the gaps between recorded regions become new ones, so that
    /// regions never overlap (review CORE-26).
    pub fn record(
        &mut self,
        start: u64,
        end: u64,
        prot: u32,
        flags: u32,
        object: Option<&'static str>,
    ) {
        let mut gaps = alloc::vec::Vec::new();
        let mut at = start;
        for (&s, r) in self.regions.range(..end).filter(|(_, r)| r.end > start) {
            if s > at {
                gaps.push((at, s));
            }
            at = at.max(r.end);
        }
        if at < end {
            gaps.push((at, end));
        }
        for (start, end) in gaps {
            self.regions.insert(
                start,
                Region {
                    end,
                    prot,
                    flags,
                    object,
                    phys: None,
                    image: false,
                    shared: None,
                    revocable: false,
                    offset: 0,
                    max_prot: PROT_ALL,
                },
            );
        }
    }
}

/// A new frame holding `bytes` (at most a page), zero-filled after them.
fn new_page(bytes: &[u8]) -> Option<u64> {
    let pa = pmem::alloc_user_page()?;
    kmem::with_frame(pa, |frame| {
        frame[..bytes.len()].copy_from_slice(bytes);
        frame[bytes.len()..].fill(0);
    });
    Some(pa)
}

// ANCHOR: apply
/// Give the pages of `[start, end)` permissions `prot`. Missing pages are
/// allocated, holding what `contents` has for them (`contents` starts at
/// `start`), unless `prot` is `PROT_NONE` and they would be zero.
fn apply(p: &Process, start: u64, end: u64, prot: u32, contents: &[u8]) -> Result<(), u32> {
    let up = user_prot(prot);
    for va in (start..end).step_by(PAGE_SIZE as usize) {
        let off = ((va - start) as usize).min(contents.len());
        let bytes = &contents[off..(off + PAGE_SIZE as usize).min(contents.len())];
        // A shared copy-on-write page about to become writable: its own
        // copy first.
        if up.write
            && let Some((pa, _)) = p.aspace().page(va)
            && pmem::sharers(pa) != 0
        {
            own_frame(p, va, pa, up)?;
        }
        let pa = match p.aspace().protect(va, up) {
            Some(pa) => pa,
            None if up == UserProt::NONE && bytes.is_empty() => continue,
            None => {
                let pa = new_page(bytes).ok_or(ENOMEM)?;
                p.aspace().map(va, pa, up).map_err(|_| ENOMEM)?;
                pa
            }
        };
        if up.exec {
            // Code the process wrote must reach the instruction cache.
            kmem::with_frame(pa, |frame| {
                mmu::sync_icache(frame.as_ptr() as u64, PAGE_SIZE)
            });
        }
    }
    Ok(())
}
// ANCHOR_END: apply

/// Private memory: `len` bytes with `prot`, at `addr` if `MAP_FIXED`
/// (replacing what was there), else wherever there is room, preferring
/// `addr`. It starts out as a copy of `contents` (a file's bytes from the
/// mapped offset on), then zeros; `object` names the file. Returns the
/// (page-aligned) address.
#[allow(clippy::too_many_arguments)]
pub fn map(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    align: u64,
    contents: &[u8],
    object: Option<&'static str>,
) -> Result<u64, u32> {
    let len = page_round(len).filter(|&l| l != 0).ok_or(EINVAL)?;
    let align = align.max(PAGE_SIZE).next_power_of_two();
    let region = Region {
        end: 0,
        prot,
        flags,
        object,
        phys: None,
        image: false,
        shared: None,
        revocable: false,
        offset: 0,
        max_prot: PROT_ALL,
    };
    let start = place(p, addr, len, flags, align, region)?;
    // A reservation: nothing to allocate, and no page to visit, however
    // large (procnto reserves 64 GB at once).
    if prot == PROT_NONE && contents.is_empty() {
        return Ok(start);
    }
    if let Err(e) = apply(p, start, start + len, prot, contents) {
        let _ = unmap(p, start, len);
        return Err(e);
    }
    Ok(start)
}

/// Where a new mapping of `len` (page-rounded) bytes goes, recorded as
/// `region` (its `end` set here): at `addr` with `MAP_FIXED` (replacing
/// what was there), else where there is room, preferring `addr`. Found
/// and recorded under one hold of the map lock, as procnto's `vmm_mmap`
/// does under the process's map lock: mappings made at the same time
/// from different threads (the manager's thread, a `SIGEV_THREAD` stack,
/// a loader, a trace buffer) can't get the same range. The caller fills
/// the pages in after, and takes the region back (`unmap`) if that
/// fails.
fn place(
    p: &Process,
    addr: u64,
    len: u64,
    flags: u32,
    align: u64,
    region: Region,
) -> Result<u64, u32> {
    if flags & MAP_FIXED != 0 {
        if !addr.is_multiple_of(PAGE_SIZE) {
            return Err(EINVAL);
        }
        // Past the end of the address space: no memory there (ENOMEM, as
        // procnto, measured).
        if addr.checked_add(len).is_none_or(|e| e > USER_END) {
            return Err(ENOMEM);
        }
        loop {
            unmap(p, addr, len)?;
            let mut map = p.mem.lock();
            if map.is_free(addr, addr + len) {
                map.regions.insert(addr, Region { end: addr + len, ..region });
                return Ok(addr);
            }
            // Mapped again meanwhile by another thread: that goes too.
        }
    }
    let mut map = p.mem.lock();
    let hint = addr & !(PAGE_SIZE - 1);
    let start = match hint.checked_add(len) {
        Some(end)
            if hint != 0
                && end <= USER_END
                && map.is_free(hint, end)
                && hint.is_multiple_of(align) =>
        {
            hint
        }
        _ => map.find_free(MMAP_BASE, len, align).ok_or(ENOMEM)?,
    };
    map.regions.insert(start, Region { end: start + len, ..region });
    Ok(start)
}

/// A private copy of memory object `obj` from page `page` on: `len`
/// bytes mapped as `map` does, then filled page by page from the frames
/// the object has (a page it hasn't reads as zeros and stays a fresh
/// zero page). No kernel buffer the size of the mapping: the object may
/// be as large as memory (review item PATHMEM-5).
#[allow(clippy::too_many_arguments)]
pub fn map_copy(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    align: u64,
    obj: &MemObject,
    page: u64,
    object: Option<&'static str>,
) -> Result<u64, u32> {
    let start = map(p, addr, len, prot, flags, align, &[], object)?;
    let exec = prot & PROT_EXEC != 0;
    for i in 0..page_round(len).unwrap_or(0) / PAGE_SIZE {
        let Some(src) = obj.present_frame(page + i) else {
            continue;
        };
        let Some((pa, _)) = p.aspace().page(start + i * PAGE_SIZE) else {
            continue;
        };
        kmem::with_frames(src, pa, |from, to| {
            to.copy_from_slice(from);
            if exec {
                mmu::sync_icache(to.as_ptr() as u64, PAGE_SIZE);
            }
        });
    }
    Ok(start)
}

/// Memory object `obj`, from page `page` of it on, mapped shared: `len`
/// bytes with `prot`, placed as for `map`; `object` names it; `max_prot`
/// is what `mprotect()` may later give it. Returns the address.
#[allow(clippy::too_many_arguments)]
pub fn map_shared(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    obj: Arc<MemObject>,
    page: u64,
    object: Option<&'static str>,
    revocable: bool,
    max_prot: u32,
) -> Result<u64, u32> {
    let len = page_round(len).filter(|&l| l != 0).ok_or(EINVAL)?;
    let region = Region {
        end: 0,
        prot,
        flags,
        object,
        phys: None,
        image: false,
        shared: Some((obj.clone(), page)),
        revocable,
        offset: page * PAGE_SIZE,
        max_prot,
    };
    let start = place(p, addr, len, flags, PAGE_SIZE, region)?;
    let up = user_prot(prot);
    for i in 0..len / PAGE_SIZE {
        let pa = match obj.frame(page + i) {
            Ok(pa) => pa,
            Err(e) => {
                let _ = unmap(p, start, len);
                return Err(e);
            }
        };
        // Uncached as asked (`PROT_NOCACHE`, as for a device's memory
        // given to the object).
        let r = if prot & PROT_NOCACHE != 0 {
            p.aspace().map_uncached(start + i * PAGE_SIZE, pa, up)
        } else {
            p.aspace().map(start + i * PAGE_SIZE, pa, up)
        };
        if r.is_err() {
            // Undo (the frames are the object's: `unmap` leaves them).
            let _ = unmap(p, start, len);
            return Err(ENOMEM);
        }
    }
    Ok(start)
}

/// Fresh contiguous RAM (a typed memory allocation, for DMA): `len` bytes
/// of frames from `pa` on, zeroed, mapped with `prot` (uncached with
/// `PROT_NOCACHE`), placed as for `map`. The frames are the process's
/// (freed when unmapped), and this function's if it fails. Returns the
/// address.
pub fn map_contig(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    pa: u64,
) -> Result<u64, u32> {
    // A length that isn't one (0, or past the end of memory) names no
    // frames that are ours to free.
    let len = page_round(len).filter(|&l| l != 0).ok_or(EINVAL)?;
    // The frames are ours from here on: on failure, what the process
    // doesn't map goes back (what it does, `unmap` frees).
    let free_from = |off: u64| {
        for o in (off..len).step_by(PAGE_SIZE as usize) {
            pmem::free_page(pa + o);
        }
    };
    let region = Region {
        end: 0,
        prot,
        flags,
        object: None,
        phys: None,
        image: false,
        shared: None,
        revocable: false,
        offset: 0,
        max_prot: PROT_ALL,
    };
    let start = match place(p, addr, len, flags, PAGE_SIZE, region) {
        Ok(start) => start,
        Err(e) => {
            free_from(0);
            return Err(e);
        }
    };
    let up = user_prot(prot);
    for off in (0..len).step_by(PAGE_SIZE as usize) {
        kmem::with_frame(pa + off, |f| {
            f.fill(0);
            // Not to be written back over what a device writes later.
            if prot & PROT_NOCACHE != 0 {
                mmu::clean_inval_dcache(f.as_ptr() as u64, PAGE_SIZE);
            }
        });
        let r = if prot & PROT_NOCACHE != 0 {
            p.aspace().map_uncached(start + off, pa + off, up)
        } else {
            p.aspace().map(start + off, pa + off, up)
        };
        if r.is_err() {
            let _ = unmap(p, start, len); // the pages mapped so far
            free_from(off); // and the rest
            return Err(ENOMEM);
        }
    }
    Ok(start)
}

/// A read-only mapping of `len` bytes of the image filesystem's
/// data at physical address `pa` (code and read-only data of its programs
/// and libraries): its pages are mapped in place, shared by every process
/// that maps them, rather than copied. `align` and placement as for `map`.
/// Returns the address.
#[allow(clippy::too_many_arguments)]
pub fn map_image(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    align: u64,
    pa: u64,
    object: Option<&'static str>,
) -> Result<u64, u32> {
    let len = page_round(len).filter(|&l| l != 0).ok_or(EINVAL)?;
    if prot & PROT_WRITE != 0 || !pa.is_multiple_of(PAGE_SIZE) {
        return Err(EINVAL);
    }
    let align = align.max(PAGE_SIZE).next_power_of_two();
    let region = Region {
        end: 0,
        prot,
        flags,
        object,
        phys: Some(pa),
        image: true,
        shared: None,
        revocable: false,
        offset: 0,
        max_prot: PROT_ALL,
    };
    let start = place(p, addr, len, flags, align, region)?;
    if let Err(e) = map_phys_pages(p, start, start + len, prot & !PROT_NOCACHE, pa) {
        let _ = unmap(p, start, len);
        return Err(e);
    }
    Ok(start)
}

/// Make the image pages of `[start, end)` the process's own copies (before
/// they become writable).
fn privatize_image(p: &Process, start: u64, end: u64) -> Result<(), u32> {
    for va in (start..end).step_by(PAGE_SIZE as usize) {
        let Some((src, prot)) = p.aspace().page(va) else {
            continue;
        };
        let pa = pmem::alloc_user_page().ok_or(ENOMEM)?;
        kmem::with_frames(src, pa, |from, to| {
            to.copy_from_slice(from);
            if prot.exec {
                mmu::sync_icache(to.as_ptr() as u64, PAGE_SIZE);
            }
        });
        p.aspace().unmap(va);
        p.aspace().map(va, pa, prot).map_err(|_| ENOMEM)?;
    }
    Ok(())
}

// ANCHOR: cow_fault
/// The page at `va` of `p`, mapped read-only, is written in a writable
/// region: a copy-on-write page shared since a fork. It becomes the
/// process's own: copied if others still map its frame, else just made
/// writable. Returns whether that was the case (else the fault is real).
pub fn cow_fault(p: &Process, addr: u64) -> bool {
    let va = addr & !(PAGE_SIZE - 1);
    let map = p.mem.lock();
    let Some((_, r)) = map.regions.range(..=va).next_back() else {
        return false;
    };
    if va >= r.end || r.prot & PROT_WRITE == 0 || r.phys.is_some() || r.shared.is_some() {
        return false;
    }
    let Some((pa, prot)) = p.aspace().page(va) else {
        return false;
    };
    if !prot.write {
        own_frame(p, va, pa, user_prot(r.prot)).is_ok()
    } else {
        true // another thread got there first
    }
}
// ANCHOR_END: cow_fault

/// Map `va` with `prot` to a frame of its own: `pa` itself if no other
/// mapping shares it, else a copy.
fn own_frame(p: &Process, va: u64, pa: u64, prot: UserProt) -> Result<(), u32> {
    loop {
        let seen = pmem::sharers(pa);
        if seen == 0 {
            p.aspace().protect(va, prot);
            return Ok(());
        }
        let copy = pmem::alloc_user_page().ok_or(ENOMEM)?;
        kmem::with_frames(pa, copy, |from, to| {
            to.copy_from_slice(from);
            if prot.exec {
                mmu::sync_icache(to.as_ptr() as u64, PAGE_SIZE);
            }
        });
        // One sharer fewer, if nobody else changed the count meanwhile.
        if pmem::unshare(pa, seen) {
            p.aspace().unmap(va);
            p.aspace().map(va, copy, prot).map_err(|_| ENOMEM)?;
            return Ok(());
        }
        pmem::free_page(copy);
    }
}

/// Make the page at `va` the process's own if it maps the image filesystem
/// in place (before the kernel writes into it for a debugger: a
/// breakpoint must not reach the other processes).
pub fn own_page(p: &Process, va: u64) -> Result<(), u32> {
    let page = va & !(PAGE_SIZE - 1);
    let mut map = p.mem.lock();
    let image = map
        .regions
        .range(..=page)
        .next_back()
        .is_some_and(|(_, r)| r.image && r.end > page);
    if !image {
        // A copy-on-write page still shared since a fork: its own copy.
        if let Some((pa, prot)) = p.aspace().page(page)
            && pmem::sharers(pa) != 0
        {
            own_frame(p, page, pa, prot)?;
        }
        return Ok(());
    }
    map.split(page);
    map.split(page + PAGE_SIZE);
    privatize_image(p, page, page + PAGE_SIZE)?;
    let r = map.regions.get_mut(&page).unwrap();
    (r.phys, r.image) = (None, false);
    Ok(())
}

/// Physical memory at `paddr` (a device's registers, `mmap_device_memory()`),
/// `len` bytes mapped with `prot` (uncached with `PROT_NOCACHE`), at `addr`
/// if `MAP_FIXED`, else wherever there is room. Returns the address.
pub fn map_phys(
    p: &Process,
    addr: u64,
    len: u64,
    prot: u32,
    flags: u32,
    paddr: u64,
) -> Result<u64, u32> {
    let len = page_round(len).filter(|&l| l != 0).ok_or(EINVAL)?;
    if !paddr.is_multiple_of(PAGE_SIZE) {
        return Err(EINVAL);
    }
    let region = Region {
        end: 0,
        prot,
        flags,
        object: None,
        phys: Some(paddr),
        image: false,
        shared: None,
        revocable: false,
        offset: 0,
        max_prot: PROT_ALL,
    };
    // Placed as for `map`, but without the hint: an address without
    // `MAP_FIXED` is ignored, as before.
    let start = place(p, if flags & MAP_FIXED != 0 { addr } else { 0 }, len, flags, PAGE_SIZE, region)?;
    if let Err(e) = map_phys_pages(p, start, start + len, prot, paddr) {
        let _ = unmap(p, start, len);
        return Err(e);
    }
    Ok(start)
}

/// Map `[start, end)` to the physical pages from `paddr` on.
fn map_phys_pages(p: &Process, start: u64, end: u64, prot: u32, paddr: u64) -> Result<(), u32> {
    let up = user_prot(prot);
    for va in (start..end).step_by(PAGE_SIZE as usize) {
        let pa = paddr + (va - start);
        let r = if prot & PROT_NOCACHE != 0 {
            p.aspace().map_device(va, pa, up)
        } else {
            p.aspace().map(va, pa, up)
        };
        r.map_err(|_| ENOMEM)?;
    }
    Ok(())
}

/// Remove the mappings in `[addr, addr + len)`.
pub fn unmap(p: &Process, addr: u64, len: u64) -> Result<(), u32> {
    let len = page_round(len).ok_or(EINVAL)?;
    let end = addr
        .checked_add(len)
        .filter(|&e| e <= USER_END)
        .ok_or(EINVAL)?;
    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(EINVAL);
    }
    let frames = remove(p, &mut p.mem.lock(), addr, end);
    free_unmapped(p, frames);
    Ok(())
}

/// Remove the mappings in `[addr, end)` (page-aligned) from `p`'s map
/// `map`, locked by the caller.
/// Returns the private frames that went, for `free_unmapped` once the map
/// is let go.
fn remove(p: &Process, map: &mut MemMap, addr: u64, end: u64) -> alloc::vec::Vec<u64> {
    map.split(addr);
    map.split(end);
    let gone: alloc::vec::Vec<u64> = map.regions.range(addr..end).map(|(&s, _)| s).collect();
    let mut frames = alloc::vec::Vec::new();
    for s in gone {
        let r = map.regions.remove(&s).unwrap();
        // Private frames (fork copies them) are free now; physical memory
        // and a memory object's frames are not ours.
        let ours = r.phys.is_none() && r.shared.is_none();
        p.aspace().unmap_range(s, r.end, |_, pa| {
            if ours {
                frames.push(pa);
            }
        });
    }
    frames
}

/// Wait until no copy into or out of `p`'s memory is at work
/// (`Process::copies`).
pub fn wait_copies(p: &Process) {
    while p.copies.load(core::sync::atomic::Ordering::SeqCst) != 0 {
        core::hint::spin_loop();
    }
}

/// Free private frames just unmapped from `p` (their TLB entries gone on
/// every CPU), once no copy into or out of `p` that may have translated
/// one before is still at work (`Process::copies`, review IPC-2: another
/// process's copy kept writing a frame that was freed and reused). Called
/// with `p`'s map unlocked: a copy may be waiting for it (`fault_in`).
/// Copies run with IRQs masked, so the wait is one copy at most.
pub fn free_unmapped(p: &Process, frames: alloc::vec::Vec<u64>) {
    if frames.is_empty() {
        return;
    }
    wait_copies(p);
    for pa in frames {
        pmem::free_page(pa);
    }
}

/// Change the protection of `[addr, addr + len)`, which must be mapped:
/// ENOMEM where it isn't, which includes a range running past the end of
/// the address space (procnto, measured).
pub fn protect(p: &Process, addr: u64, len: u64, prot: u32) -> Result<(), u32> {
    let len = page_round(len).ok_or(ENOMEM)?;
    let end = addr
        .checked_add(len)
        .filter(|&e| e <= USER_END)
        .ok_or(ENOMEM)?;
    if !addr.is_multiple_of(PAGE_SIZE) {
        return Err(EINVAL);
    }
    {
        let mut map = p.mem.lock();
        // Every byte must be covered by a region.
        let mut at = addr;
        for (&s, r) in map.regions.range(..end) {
            if r.end <= at {
                continue;
            }
            if s > at {
                break;
            }
            at = r.end;
        }
        if at < end {
            return Err(ENOMEM);
        }
        // A mapping keeps its descriptor's cap (`max_prot`).
        if map.regions.range(..end).any(|(_, r)| r.end > addr && prot & !r.max_prot != 0) {
            return Err(crate::ker::errno::EACCES);
        }
        map.split(addr);
        map.split(end);
        // Image pages becoming writable: the process's own copies from now.
        if prot & PROT_WRITE != 0 {
            let image: alloc::vec::Vec<(u64, u64)> = map
                .regions
                .range(addr..end)
                .filter(|(_, r)| r.image)
                .map(|(&s, r)| (s, r.end))
                .collect();
            for (s, e) in image {
                privatize_image(p, s, e)?;
                let r = map.regions.get_mut(&s).unwrap();
                (r.phys, r.image) = (None, false);
            }
        }
        for (_, r) in map.regions.range_mut(addr..end) {
            r.prot = prot;
        }
    }
    apply(p, addr, end, prot, &[])
}

/// Copy `parent`'s memory into `child` (for `fork()`): the same regions,
/// each private page copied into a new frame with the same permissions
/// (with `-mc`, shared copy-on-write); device memory and memory objects'
/// frames are shared.
pub fn fork(parent: &Process, child: &Process) -> Result<(), u32> {
    let cow = COW_FORK.load(core::sync::atomic::Ordering::Relaxed);
    // Neither are mappings made MAP_NOINHERIT (procnto maps a logger's
    // trace buffers so), nor revocable ones (procnto marks them
    // MAP_NOINHERIT): the creator couldn't take them back from a child it
    // doesn't know.
    let regions: alloc::vec::Vec<(u64, Region)> = parent
        .mem
        .lock()
        .regions
        .iter()
        .filter(|(_, r)| !r.revocable && r.flags & MAP_NOINHERIT == 0)
        .map(|(&s, r)| (s, r.clone()))
        .collect();
    // Recorded in the child before its pages exist, so that a copy that
    // fails midway (out of memory) is undone by `clear(child)`: it frees
    // the frames mapped so far (one share each of copy-on-write ones).
    child.mem.lock().regions.extend(regions.iter().cloned());
    // Copy-on-write: every private page of the parent read-only and its
    // TLB flushed, on every CPU, before the child maps any of them; else
    // a parent thread elsewhere keeps writing through its writable TLB
    // entry into what the child already has, and the child's memory
    // isn't a snapshot. The frames are shared from here on: on failure,
    // the shares of the pages the child never mapped are given back.
    let mut snapshot: alloc::vec::Vec<(u64, u64, UserProt)> = alloc::vec::Vec::new();
    if cow {
        for (start, r) in &regions {
            if r.phys.is_some() || r.shared.is_some() {
                continue;
            }
            for va in (*start..r.end).step_by(PAGE_SIZE as usize) {
                if let Some((src, prot)) = parent.aspace().write_protect_batched(va) {
                    pmem::share(src);
                    snapshot.push((va, src, prot));
                }
            }
        }
        parent.aspace().flush();
    }
    let mut mapped = 0;
    let r = fork_pages(parent, child, &regions, cow, &snapshot, &mut mapped);
    if r.is_err() {
        for &(_, src, _) in &snapshot[mapped..] {
            pmem::free_page(src); // one share fewer
        }
    }
    r
}

/// The pages of `regions` into the child: a device's as they are, a
/// memory object's shared, private ones copied, or (`cow`) the frames of
/// `snapshot` read-only in both; `*mapped` counts the snapshot entries
/// the child has.
fn fork_pages(
    parent: &Process,
    child: &Process,
    regions: &[(u64, Region)],
    cow: bool,
    snapshot: &[(u64, u64, UserProt)],
    mapped: &mut usize,
) -> Result<(), u32> {
    for (start, r) in regions {
        let start = *start;
        if let Some(paddr) = r.phys {
            // A device's registers: the child maps the same ones.
            map_phys_pages(child, start, r.end, r.prot, paddr)?;
            continue;
        }
        if r.shared.is_some() {
            // A memory object's frames: shared, not copied.
            for va in (start..r.end).step_by(PAGE_SIZE as usize) {
                let Some((src, prot)) = parent.aspace().page(va) else {
                    continue;
                };
                child.aspace().map(va, src, prot).map_err(|_| ENOMEM)?;
            }
            continue;
        }
        if cow {
            continue; // below, from the snapshot
        }
        for va in (start..r.end).step_by(PAGE_SIZE as usize) {
            // A private page: the child's own copy, now.
            let Some((src, prot)) = parent.aspace().page(va) else {
                continue;
            };
            let pa = pmem::alloc_user_page().ok_or(ENOMEM)?;
            kmem::with_frames(src, pa, |from, to| {
                to.copy_from_slice(from);
                if prot.exec {
                    mmu::sync_icache(to.as_ptr() as u64, PAGE_SIZE);
                }
            });
            if child.aspace().map(va, pa, prot).is_err() {
                pmem::free_page(pa);
                return Err(ENOMEM);
            }
        }
    }
    // A private page under copy-on-write: shared read-only by both until
    // one writes it (see `cow_fault`).
    for &(va, src, prot) in snapshot {
        let ro = UserProt {
            write: false,
            ..prot
        };
        child.aspace().map(va, src, ro).map_err(|_| ENOMEM)?;
        *mapped += 1;
    }
    Ok(())
}

/// Make `len` bytes of `p`'s memory from `start` a stack's guard: no
/// pages, no access (SEGV_MAPERR), kept as a stack region so that nothing
/// else is placed there and `/proc` counts it in the stack. Under one
/// lock of the map: another thread's mapping placed in between would
/// otherwise get the guard's range and lose its record to it.
pub fn make_guard(p: &Process, start: u64, len: u64) {
    let mut map = p.mem.lock();
    let frames = remove(p, &mut map, start, start + len);
    map.record(start, start + len, PROT_NONE, MAP_PRIVATE | MAP_ANON | MAP_STACK, None);
    drop(map);
    free_unmapped(p, frames);
}

/// `shm_revoke()`: remove `p`'s revocable mappings of `obj`; touching
/// them then faults as unmapped memory (SIGSEGV, `SEGV_MAPERR`).
pub fn revoke(p: &Process, obj: &Arc<MemObject>) {
    let gone: alloc::vec::Vec<(u64, u64)> = p
        .mem
        .lock()
        .regions
        .iter()
        .filter(|(_, r)| r.revocable && r.shared.as_ref().is_some_and(|(o, _)| Arc::ptr_eq(o, obj)))
        .map(|(&s, r)| (s, r.end - s))
        .collect();
    for (start, len) in gone {
        let _ = unmap(p, start, len);
    }
}

/// Remove `p`'s mappings of the physical range `[pa, pa + len)`, frames
/// the kernel takes back (old trace buffers); touching them then faults
/// as unmapped memory. If it had any, returns once no kernel copy into or
/// out of `p`'s memory is at work, so none still reaches those frames.
pub fn unmap_phys(p: &Process, pa: u64, len: u64) {
    let top = pa + len;
    let gone: alloc::vec::Vec<(u64, u64)> = p
        .mem
        .lock()
        .regions
        .iter()
        .filter_map(|(&s, r)| {
            let base = r.phys?;
            let (lo, hi) = (base.max(pa), (base + (r.end - s)).min(top));
            (lo < hi).then(|| (s + (lo - base), hi - lo))
        })
        .collect();
    if gone.is_empty() {
        return;
    }
    for (start, len) in gone {
        let _ = unmap(p, start, len);
    }
    wait_copies(p);
}

/// A fault on `addr` of `p` that the memory manager resolves: only a
/// write to a page shared copy-on-write since a fork (`-mc`); as QNX 8's
/// memory is superlocked, nothing else is faulted in. Returns true if the
/// access can be retried. Called on a permission or translation fault from
/// EL0 and when the kernel accesses user memory.
pub fn fault_in(p: &Process, addr: u64, write: bool) -> bool {
    write && cow_fault(p, addr)
}

/// Remove every mapping of `p` (for `exec()`, and when it has ended).
pub fn clear(p: &Process) {
    let starts: alloc::vec::Vec<(u64, u64)> = p
        .mem
        .lock()
        .regions
        .iter()
        .map(|(&s, r)| (s, r.end))
        .collect();
    for (start, end) in starts {
        let _ = unmap(p, start, end - start);
    }
}

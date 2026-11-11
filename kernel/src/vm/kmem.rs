//! Kernel virtual memory: fresh pages mapped into the kernel address space.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::mmu::{self, BLOCK_SIZE, LINEAR_BASE, LINEAR_MAX, Prot, UNUSED_BASE};
use crate::vm::pmem::{self, PAGE_SIZE};

/// Next free kernel virtual address. Starts above the mmu self-test page.
static NEXT_VA: AtomicU64 = AtomicU64::new(UNUSED_BASE + (2 << 20));

/// Longest range (in pages) whose addresses are reused.
const REUSE_MAX: usize = 64;

/// The kernel addresses handed out so far, `[start, end)` (heap blocks,
/// stacks, windows: the kernel's own memory, for a crash dump).
pub fn handed_out() -> (u64, u64) {
    (UNUSED_BASE + (2 << 20), NEXT_VA.load(Ordering::Relaxed))
}

/// Kernel address ranges given back (`unmap_new`), by length in pages:
/// reused by the next mapping of that length, so that the kernel's page
/// tables stop growing with threads and large heap blocks.
static FREE_VA: [crate::sync::IrqSpinLock<alloc::vec::Vec<u64>>; REUSE_MAX + 1] =
    [const { crate::sync::IrqSpinLock::new(alloc::vec::Vec::new()) }; REUSE_MAX + 1];

/// `pages` pages of kernel address space: a range given back, or new.
fn take_va(pages: u64) -> u64 {
    FREE_VA
        .get(pages as usize)
        .and_then(|b| b.lock().pop())
        .unwrap_or_else(|| NEXT_VA.fetch_add(pages * PAGE_SIZE, Ordering::Relaxed))
}

/// Keep `va` for reuse. The list grows outside its lock (growing
/// allocates, which may take a range from it); the old one is freed
/// after.
fn give_va(va: u64, pages: u64) {
    let Some(bucket) = FREE_VA.get(pages as usize) else {
        return;
    };
    loop {
        let need = {
            let mut list = bucket.lock();
            if list.len() < list.capacity() {
                list.push(va);
                return;
            }
            list.capacity() * 2 + 16
        };
        let mut bigger = alloc::vec::Vec::with_capacity(need);
        {
            let mut list = bucket.lock();
            if list.capacity() >= need {
                continue;
            }
            bigger.extend(list.drain(..));
            core::mem::swap(&mut *list, &mut bigger);
        }
        drop(bigger);
    }
}

/// Unmap and free the frames of `[va, va + pages)` (from `map_new`; pages
/// not mapped are skipped); its addresses are reused. Each TLB entry is
/// gone on every CPU once this returns.
pub fn unmap_new(va: u64, pages: u64) {
    mmu::unmap_pages(va, pages, pmem::free_page);
    give_va(va, pages);
}

/// Unmap and free what `map_new_guarded(pages)` mapped at `va`; its
/// addresses are reused.
pub fn unmap_guarded(va: u64, pages: u64) {
    unmap_new(va - PAGE_SIZE, pages + 1);
}

/// The ranges of physical memory in the linear map, as (start, end).
static LINEAR: [(AtomicU64, AtomicU64); 8] = [const { (AtomicU64::new(0), AtomicU64::new(0)) }; 8];

/// Map RAM (the syspage's `ram` ranges, in whole 2 MiB blocks) at
/// `LINEAR_BASE` + its physical address, so that the kernel reaches any
/// frame without a window. Returns the bytes mapped. A block that holds
/// the kernel's code or read-only data is mapped with pages instead,
/// those read-only: no writable alias of the kernel's text.
pub fn init_linear(sp: &crate::syspage::Syspage) -> u64 {
    let ro = mmu::kernel_ro_phys();
    if ro.is_none() {
        crate::kprintln!("kmem: the kernel image isn't contiguous: its text stays writable in the linear map");
    }
    let (ro_start, ro_end) = ro.unwrap_or((0, 0));
    let map = |pa: u64| -> bool {
        if pa + BLOCK_SIZE <= ro_start || pa >= ro_end {
            return mmu::map_block(LINEAR_BASE + pa, pa, Prot::Data).is_ok();
        }
        (pa..pa + BLOCK_SIZE).step_by(PAGE_SIZE as usize).all(|p| {
            let prot = if (ro_start..ro_end).contains(&p) { Prot::ReadOnly } else { Prot::Data };
            mmu::map_page(LINEAR_BASE + p, p, prot).is_ok()
        })
    };
    let mut n = 0;
    let mut bytes = 0;
    for a in sp.asinfo().filter(|a| sp.string(a.name) == "ram") {
        let start = a.start.next_multiple_of(BLOCK_SIZE);
        let end = ((a.end + 1) & !(BLOCK_SIZE - 1)).min(LINEAR_MAX);
        if end <= start || n == LINEAR.len() {
            continue;
        }
        let mut pa = start;
        while pa < end {
            if !map(pa) {
                break;
            }
            pa += BLOCK_SIZE;
        }
        LINEAR[n].0.store(start, Ordering::Relaxed);
        LINEAR[n].1.store(pa, Ordering::Release);
        n += 1;
        bytes += pa - start;
    }
    if let Some(va) = ro.and_then(|(start, _)| linear(start)) {
        let state = if mmu::writable(va) { "writable" } else { "read-only" };
        crate::kprintln!("kmem: the kernel's text is {state} in the linear map");
    }
    bytes
}

/// The kernel address of physical address `pa` in the linear map, if RAM.
#[inline]
pub fn linear(pa: u64) -> Option<u64> {
    LINEAR
        .iter()
        .any(|(s, e)| (s.load(Ordering::Relaxed)..e.load(Ordering::Acquire)).contains(&pa))
        .then_some(LINEAR_BASE + pa)
}

/// Map `pages` new zeroed frames at a new kernel address, with an unmapped
/// guard page below them (for stacks).
pub fn map_new_guarded(pages: u64, prot: Prot) -> Option<u64> {
    let base = take_va(pages + 1);
    // Out of memory part-way: what was mapped is freed and the range given
    // back (review CORE-14: a thread whose stack failed leaked them).
    map_at(base + PAGE_SIZE, pages, prot).or_else(|| {
        unmap_new(base, pages + 1);
        None
    })
}

/// Reserve `pages` of kernel address space without mapping them.
pub fn reserve(pages: u64) -> Option<u64> {
    Some(NEXT_VA.fetch_add(pages * PAGE_SIZE, Ordering::Relaxed))
}

/// Map the physical range `[pa, pa + len)` at a new kernel address with
/// `prot`; returns the address of `pa`. Out of memory part-way, what was
/// mapped is undone.
pub fn map_phys(pa: u64, len: u64, prot: Prot) -> Option<u64> {
    let first = pa & !(PAGE_SIZE - 1);
    let pages = (pa + len - first).div_ceil(PAGE_SIZE);
    let va = NEXT_VA.fetch_add(pages * PAGE_SIZE, Ordering::Relaxed);
    for i in 0..pages {
        if mmu::map_page(va + i * PAGE_SIZE, first + i * PAGE_SIZE, prot).is_err() {
            unmap_phys(va, i * PAGE_SIZE);
            return None;
        }
    }
    Some(va + (pa - first))
}

/// Unmap what `map_phys(pa, len)` mapped at `va`, leaving the frames to
/// whoever owns them; its addresses are reused. Each TLB entry is gone on
/// every CPU once this returns.
pub fn unmap_phys(va: u64, len: u64) {
    let first = va & !(PAGE_SIZE - 1);
    let pages = (va + len - first).div_ceil(PAGE_SIZE);
    if pages == 0 {
        return;
    }
    mmu::unmap_pages(first, pages, |_| {});
    give_va(first, pages);
}

/// Reserve this CPU's frame windows (two unmapped kernel pages).
pub fn init_cpu() {
    let va = reserve(2).expect("frame windows");
    crate::percpu::this()
        .frame_window
        .store(va, Ordering::Relaxed);
}

/// Run `f` on page frame `pa`, mapped at one of this CPU's frame windows
/// (with interrupts masked, so the window stays ours). For short work on
/// frames that have no kernel mapping (user pages); unlike `map_frame`,
/// it uses no kernel address space. One slice: not `with_frames(pa, pa)`,
/// which would make two `&mut` to the same memory (review CORE-9).
pub fn with_frame<R>(pa: u64, f: impl FnOnce(&mut [u8]) -> R) -> R {
    let frame = pa & !(PAGE_SIZE - 1);
    if let Some(l) = linear(frame) {
        // SAFETY: the frame is RAM, mapped for good in the linear map.
        let s = unsafe { core::slice::from_raw_parts_mut(l as *mut u8, PAGE_SIZE as usize) };
        return f(s);
    }
    let daif = crate::arch::irq_save();
    let va = window(0, frame);
    // SAFETY: the window maps the whole frame for the duration of `f`; with
    // interrupts masked nothing else on this CPU uses it.
    let s = unsafe { core::slice::from_raw_parts_mut(va as *mut u8, PAGE_SIZE as usize) };
    let r = f(s);
    crate::arch::irq_restore(daif);
    r
}

/// Run `f` on two different page frames at once (copying a page), mapped
/// at this CPU's two frame windows.
pub fn with_frames<R>(a: u64, b: u64, f: impl FnOnce(&mut [u8], &mut [u8]) -> R) -> R {
    let (fa, fb) = (a & !(PAGE_SIZE - 1), b & !(PAGE_SIZE - 1));
    // Two `&mut` to one frame would alias (review CORE-9).
    assert_ne!(fa, fb, "with_frames on one frame");
    if let (Some(la), Some(lb)) = (linear(fa), linear(fb)) {
        // SAFETY: both frames are RAM, mapped for good in the linear map.
        let sa = unsafe { core::slice::from_raw_parts_mut(la as *mut u8, PAGE_SIZE as usize) };
        // SAFETY: as above; another frame, so the slices don't overlap.
        let sb = unsafe { core::slice::from_raw_parts_mut(lb as *mut u8, PAGE_SIZE as usize) };
        return f(sa, sb);
    }
    let daif = crate::arch::irq_save();
    let (wa, wb) = (window(0, fa), window(1, fb));
    // SAFETY: both windows map whole frames for the duration of `f`; with
    // interrupts masked nothing else on this CPU uses them.
    let sa = unsafe { core::slice::from_raw_parts_mut(wa as *mut u8, PAGE_SIZE as usize) };
    // SAFETY: as above; another window and another frame.
    let sb = unsafe { core::slice::from_raw_parts_mut(wb as *mut u8, PAGE_SIZE as usize) };
    let r = f(sa, sb);
    crate::arch::irq_restore(daif);
    r
}

/// This CPU's frame window `i` (0 or 1), mapping `frame` as data:
/// remapped only when it maps another. Interrupts must be masked.
fn window(i: usize, frame: u64) -> u64 {
    window_as(i, frame, Prot::Data)
}

/// `window`, mapping `frame` with `prot`. What the window maps is
/// remembered with its kind (bit 0 set for a device), so that one frame
/// is never reached through a stale mapping of the other kind.
fn window_as(i: usize, frame: u64, prot: Prot) -> u64 {
    let cpu = crate::percpu::this();
    let va = cpu.frame_window.load(Ordering::Relaxed) + i as u64 * PAGE_SIZE;
    let key = frame | (prot == Prot::Device) as u64;
    if cpu.frame_window_pa[i].load(Ordering::Relaxed) != key {
        mmu::set_window(va, frame, prot).expect("frame window");
        cpu.frame_window_pa[i].store(key, Ordering::Relaxed);
    }
    va
}

/// Run `f` on frame `pa` of a device (memory outside RAM), mapped as
/// device memory at frame window 0 with interrupts masked; `f` gets its
/// address. Not a slice: a device's registers are accessed explicitly
/// (`arch::exception::fault_copy`), never through a Rust reference.
pub fn with_device_frame<R>(pa: u64, f: impl FnOnce(*mut u8) -> R) -> R {
    let daif = crate::arch::irq_save();
    let va = window_as(0, pa & !(PAGE_SIZE - 1), Prot::Device);
    let r = f(va as *mut u8);
    crate::arch::irq_restore(daif);
    r
}

/// Map an existing page frame at a new kernel address (for good: kernel
/// address space is not reused).
pub fn map_frame(pa: u64) -> Option<u64> {
    let va = NEXT_VA.fetch_add(PAGE_SIZE, Ordering::Relaxed);
    mmu::map_page(va, pa, Prot::Data).ok()?;
    Some(va)
}

/// Map `pages` new zeroed frames at a new kernel address.
pub fn map_new(pages: u64, prot: Prot) -> Option<u64> {
    let va = take_va(pages);
    // Out of memory part-way: undone (`map_new_guarded`).
    map_at(va, pages, prot).or_else(|| {
        unmap_new(va, pages);
        None
    })
}

fn map_at(va: u64, pages: u64, prot: Prot) -> Option<u64> {
    for i in 0..pages {
        let pa = pmem::alloc_page()?;
        let page = va + i * PAGE_SIZE;
        if mmu::map_page(page, pa, Prot::Data).is_err() {
            pmem::free_page(pa);
            return None;
        }
        // SAFETY: the page was just mapped writable and belongs to us.
        unsafe { core::ptr::write_bytes(page as *mut u8, 0, PAGE_SIZE as usize) };
        if prot != Prot::Data {
            mmu::protect(page, prot).ok()?;
        }
    }
    Some(va)
}

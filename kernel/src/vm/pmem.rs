//! Physical memory: page frames from the RAM that startup left free.
//!
//! Startup describes free RAM as `sysram` entries in the system page's
//! address-space tree (asinfo). Pages are handed out from those ranges in
//! order; freed pages go on a stack that is used first. The stack has room
//! for every page from the start (`init_free_list`), so freeing never
//! allocates, which matters as the heap gets its own pages from here.
//! Frame 0 is never handed out: it would look like a null address, and a
//! board's firmware may keep data there.

use alloc::vec::Vec;

use core::sync::atomic::{AtomicU16, Ordering};

use crate::sync::{InitOnce, IrqSpinLock};
use crate::syspage::Syspage;

pub const PAGE_SIZE: u64 = 4096;
const MAX_RANGES: usize = 32;

#[derive(Debug, Clone, Copy, Default)]
struct Range {
    /// Its first page (at boot): frames from here to `end` are RAM the
    /// kernel hands out, and only those may be freed.
    start: u64,
    /// Next free page.
    next: u64,
    /// One past the last byte.
    end: u64,
}

struct Pmem {
    ranges: [Range; MAX_RANGES],
    count: usize,
}

/// Masks interrupts: interrupt handlers allocate too (a heap refill), and
/// one spinning on a lock its own CPU holds would never get it.
static PMEM: IrqSpinLock<Pmem> = IrqSpinLock::new(Pmem {
    ranges: [Range { start: 0, next: 0, end: 0 }; MAX_RANGES],
    count: 0,
});

/// Bytes of RAM that were free at boot.
static TOTAL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Bytes of RAM there are to hand out (free at boot).
pub fn total_bytes() -> u64 {
    TOTAL.load(core::sync::atomic::Ordering::Relaxed)
}

/// Collect the `sysram` ranges. Returns (number of ranges, free bytes).
pub fn init(sp: &Syspage) -> (usize, u64) {
    let mut p = PMEM.lock();
    p.count = 0;
    let mut total = 0;
    for a in sp.asinfo().filter(|a| sp.string(a.name) == "sysram") {
        let next = a.start.max(PAGE_SIZE).next_multiple_of(PAGE_SIZE);
        let end = (a.end + 1) & !(PAGE_SIZE - 1);
        if end <= next {
            continue;
        }
        if p.count == MAX_RANGES {
            crate::kprintln!("pmem: more than {MAX_RANGES} sysram ranges; ignoring the rest");
            break;
        }
        let i = p.count;
        p.ranges[i] = Range { start: next, next, end };
        p.count += 1;
        total += end - next;
    }
    TOTAL.store(total, core::sync::atomic::Ordering::Relaxed);
    (p.count, total)
}

/// Extra mappings of each frame (copy-on-write pages after fork: a frame
/// with n sharers has n - 1 here), by frame number. A frame goes back to
/// the free stack when its last mapping does.
static SHARES: InitOnce<alloc::boxed::Box<[AtomicU16]>> = InitOnce::new();

fn shares(pa: u64) -> Option<&'static AtomicU16> {
    SHARES.get()?.get((pa / PAGE_SIZE) as usize)
}

/// One more mapping of frame `pa` (shared by fork).
pub fn share(pa: u64) {
    if let Some(s) = shares(pa) {
        s.fetch_add(1, Ordering::AcqRel);
    }
}

/// How many other mappings frame `pa` has.
pub fn sharers(pa: u64) -> u16 {
    shares(pa).map_or(0, |s| s.load(Ordering::Acquire))
}

/// Drop one of the `seen` other mappings of `pa` (the caller has made
/// its own copy); false if the count changed meanwhile (look again).
pub fn unshare(pa: u64, seen: u16) -> bool {
    shares(pa).is_some_and(|s| {
        s.compare_exchange(seen, seen - 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    })
}

/// Free page frames: a stack of freed ones, used before the untouched
/// ranges, and a bitmap of every free frame (freed or untouched), which
/// contiguous allocations search (review TOOLS-25: they used to take only
/// untouched RAM, so once memory had been used and freed, a 1 MiB
/// `mmap(MAP_PHYS | MAP_ANON)` failed with ENOMEM though most of memory
/// was free, and io-sock, which allocates its buffers so, stalled).
///
/// A frame a contiguous allocation takes stays on the stack as a stale
/// entry, skipped when it comes up; `stacked` marks the frames with an
/// entry, so that a frame is never on the stack twice and the stack's
/// room for every frame suffices.
struct Free {
    stack: Vec<u64>,
    /// Bit n: frame n is free.
    free: Vec<u64>,
    /// Bit n: frame n has an entry on the stack (maybe stale).
    stacked: Vec<u64>,
    /// Free frames (the bits set in `free`).
    count: usize,
    /// The RAM ranges, as frame numbers `[first, end)`: a frame outside
    /// them is not ours to free (review CORE-25).
    ram: Vec<(u64, u64)>,
    /// The bitmaps exist (`init_free_list`, once the heap does).
    ready: bool,
}

static FREE: IrqSpinLock<Free> = IrqSpinLock::new(Free {
    stack: Vec::new(),
    free: Vec::new(),
    stacked: Vec::new(),
    count: 0,
    ram: Vec::new(),
    ready: false,
});

fn bit(v: &[u64], i: usize) -> bool {
    v.get(i / 64).is_some_and(|w| w & 1 << (i % 64) != 0)
}

fn set_bit(v: &mut [u64], i: usize, on: bool) {
    if let Some(w) = v.get_mut(i / 64) {
        if on {
            *w |= 1 << (i % 64);
        } else {
            *w &= !(1 << (i % 64));
        }
    }
}

/// Make room in the free stack for every page and build the bitmaps
/// (needs the heap).
pub fn init_free_list() {
    // The stack has room for every RAM frame, those the kernel took before
    // now included: one freed later goes on it too. The ranges are copied
    // out first: allocating under PMEM's lock deadlocks when the heap
    // grows, as growing takes a page from here (review CORE-18: the heap's
    // self-test used to grow it beforehand).
    let (ranges, n) = {
        let p = PMEM.lock();
        (p.ranges, p.count)
    };
    let r = &ranges[..n];
    let pages = r.iter().map(|r| (r.end - r.start) / PAGE_SIZE).sum::<u64>();
    let frames = (r.iter().map(|r| r.end).max().unwrap_or(0) / PAGE_SIZE) as usize;
    let ram = r.iter().map(|r| (r.start / PAGE_SIZE, r.end / PAGE_SIZE)).collect::<Vec<_>>();
    // Everything allocated first: the heap grows by taking untouched
    // pages, so which pages are untouched is read only after.
    let stack = Vec::with_capacity(pages as usize);
    let mut free = alloc::vec![0u64; frames.div_ceil(64)];
    let stacked = alloc::vec![0u64; frames.div_ceil(64)];
    let _ = SHARES.set((0..frames).map(|_| AtomicU16::new(0)).collect());
    let mut count = 0;
    {
        let p = PMEM.lock();
        for r in &p.ranges[..p.count] {
            for f in r.next / PAGE_SIZE..r.end / PAGE_SIZE {
                set_bit(&mut free, f as usize, true);
                count += 1;
            }
        }
    }
    *FREE.lock() = Free { stack, free, stacked, count, ram, ready: true };
}

/// Give a page frame back (one mapping of it, if it is shared).
pub fn free_page(pa: u64) {
    if shares(pa).is_some_and(|s| {
        s.fetch_update(Ordering::AcqRel, Ordering::Acquire, |c| c.checked_sub(1))
            .is_ok()
    }) {
        return;
    }
    let pa = pa & !(PAGE_SIZE - 1);
    let mut f = FREE.lock();
    let i = (pa / PAGE_SIZE) as usize;
    // Before the bitmaps exist (early boot) the stack has no room: leaked.
    if !f.ready {
        return;
    }
    // A frame that isn't RAM we hand out, or that is free already, is a
    // caller's bug: counted or handed out, it would corrupt the free
    // memory (a device frame given to a process, a frame handed out
    // twice). It is left alone, and said in a debug build (review CORE-25).
    let ram = f.ram.iter().any(|&(first, end)| (first..end).contains(&(i as u64)));
    if !ram || bit(&f.free, i) {
        drop(f);
        if cfg!(debug_assertions) {
            let why = if ram { "already free" } else { "not RAM the kernel hands out" };
            crate::kprintln!("pmem: free of {pa:#x} ignored: {why}");
        }
        return;
    }
    set_bit(&mut f.free, i, true);
    f.count += 1;
    if !bit(&f.stacked, i) && f.stack.len() < f.stack.capacity() {
        f.stack.push(pa);
        set_bit(&mut f.stacked, i, true);
    }
}

/// Free page frames.
pub fn free_pages() -> usize {
    {
        let f = FREE.lock();
        if f.ready {
            return f.count;
        }
    }
    let p = PMEM.lock();
    p.ranges[..p.count]
        .iter()
        .map(|r| ((r.end - r.next) / PAGE_SIZE) as usize)
        .sum()
}

/// The last physical address there is: the end of the system page's
/// "memory" address-space entry (the root one, without an owner), which
/// bounds the physical memory a process may map (procnto, measured:
/// `mmap_device_memory()` and `shm_ctl(SHMCTL_PHYS)` past it are EINVAL).
pub fn phys_end() -> u64 {
    let sp = crate::syspage::get();
    sp.asinfo()
        .find(|a| sp.string(a.name) == "memory")
        .or_else(|| sp.asinfo().find(|a| a.owner == crate::syspage::AS_NULL_OFF))
        .map_or(u64::MAX, |a| a.end)
}

/// Whether `pages` more pages could be given to a process without eating
/// the kernel's reserve (what a load about to back an image asks).
pub fn can_spare(pages: u64) -> bool {
    usize::try_from(pages).is_ok_and(|n| free_pages().saturating_sub(n) > RESERVE_PAGES)
}

/// `pages` physically contiguous page frames (not zeroed) between `lo`
/// and `hi` (inclusive), for DMA, typed memory, shared-memory objects:
/// the lowest run of free frames that fits, freed ones included. Returns
/// the first.
pub fn alloc_contig(pages: u64, lo: u64, hi: u64) -> Option<u64> {
    pages.checked_mul(PAGE_SIZE)?;
    // For a process (typed memory, a shared-memory object, a trace
    // buffer): the kernel's reserve stays.
    if pages == 0 || free_pages().saturating_sub(pages as usize) <= RESERVE_PAGES {
        return None;
    }
    let mut f = FREE.lock();
    if !f.ready {
        return None;
    }
    let first = lo.div_ceil(PAGE_SIZE);
    let limit = hi.saturating_add(1) / PAGE_SIZE; // frames below this
    let frames = (f.free.len() * 64) as u64;
    let (mut run_start, mut run) = (first, 0u64);
    let mut fr = first;
    while fr < limit.min(frames) {
        // Whole free or used words at a time.
        let w = f.free[(fr / 64) as usize];
        if fr.is_multiple_of(64) && w == 0 {
            run = 0;
            fr += 64;
            run_start = fr;
            continue;
        }
        if bit(&f.free, fr as usize) {
            if run == 0 {
                run_start = fr;
            }
            run += 1;
            if run == pages {
                break;
            }
        } else {
            run = 0;
        }
        fr += 1;
    }
    if run < pages {
        return None;
    }
    for i in run_start..run_start + pages {
        set_bit(&mut f.free, i as usize, false);
    }
    f.count -= pages as usize;
    Some(run_start * PAGE_SIZE)
}

/// The free memory between `lo` and `hi` (inclusive), in bytes: what a
/// typed memory pool there can still allocate, scattered.
pub fn free_in(lo: u64, hi: u64) -> u64 {
    let f = FREE.lock();
    let first = lo.div_ceil(PAGE_SIZE);
    let limit = (hi.saturating_add(1) / PAGE_SIZE).min((f.free.len() * 64) as u64);
    (first..limit).filter(|&fr| bit(&f.free, fr as usize)).count() as u64 * PAGE_SIZE
}

/// The largest run of free frames between `lo` and `hi`, in bytes (what
/// `alloc_contig` could still give).
pub fn largest_contig(lo: u64, hi: u64) -> u64 {
    let f = FREE.lock();
    let first = lo.div_ceil(PAGE_SIZE);
    let limit = (hi.saturating_add(1) / PAGE_SIZE).min((f.free.len() * 64) as u64);
    let (mut best, mut run) = (0u64, 0u64);
    for fr in first..limit {
        if bit(&f.free, fr as usize) {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    best * PAGE_SIZE
}

// ANCHOR: pmem_alloc
/// One page frame (not zeroed): a freed one first, else the next untouched
/// one; either may have gone to a contiguous allocation meanwhile.
pub fn alloc_page() -> Option<u64> {
    {
        let mut f = FREE.lock();
        while let Some(pa) = f.stack.pop() {
            let i = (pa / PAGE_SIZE) as usize;
            set_bit(&mut f.stacked, i, false);
            if bit(&f.free, i) {
                set_bit(&mut f.free, i, false);
                f.count -= 1;
                return Some(pa);
            }
        }
    }
    let mut p = PMEM.lock();
    let count = p.count;
    for r in &mut p.ranges[..count] {
        while r.next < r.end {
            let page = r.next;
            r.next += PAGE_SIZE;
            let mut f = FREE.lock();
            if !f.ready {
                return Some(page);
            }
            let i = (page / PAGE_SIZE) as usize;
            if bit(&f.free, i) {
                set_bit(&mut f.free, i, false);
                f.count -= 1;
                return Some(page);
            }
        }
    }
    None
}
// ANCHOR_END: pmem_alloc

/// Page frames kept from processes for the kernel: its heap, its page
/// tables, its thread stacks. A process that takes all of memory
/// (`mmap()`, a fork's copy) then fails with ENOMEM, as on procnto,
/// instead of starving the kernel, whose own allocation failure is a
/// panic (`handle_alloc_error`).
const RESERVE_PAGES: usize = 4096; // 16 MiB

/// May a process's request make the kernel take `pages` more frames (its
/// queued signals, for one), leaving the kernel's reserve whole?
pub fn user_may_take(pages: usize) -> bool {
    free_pages() > RESERVE_PAGES + pages
}

/// One page frame for a process's memory (not zeroed): none once only
/// the kernel's reserve is left.
pub fn alloc_user_page() -> Option<u64> {
    if free_pages() <= RESERVE_PAGES {
        return None;
    }
    alloc_page()
}

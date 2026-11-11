//! The pages under FreeBSD's `kmem_*` and UMA, in the shape of FreeBSD's own
//! kernel memory: a large address range reserved at startup, using no
//! memory, and backed in chunks on demand (`kmem_back`), with a `vm_page`
//! per page beside it. `PROT_NONE` reserves, as on procnto (test `reserve`).
//!
//! An address is its own "physical" address (`kern/mem.c`), and
//! `PHYS_TO_VM_PAGE()` an index, so that UMA needs no change. `io-sock` keeps
//! a hash from address to slab instead (`qnx_slabop`).

use core::ffi::{c_int, c_void};
use std::sync::Mutex;

const PAGE: usize = 4096;
const CHUNK_PAGES: usize = 512;
/// The reserved range: an address limit, not memory.
const RESERVED_PAGES: usize = 1 << 24;
const PROT_NONE: c_int = 0;
const PROT_RW: c_int = 3;
const MAP_PRIVATE: c_int = 2;
const MAP_ANON: c_int = 0x1000;

unsafe extern "C" {
    fn mmap(a: *mut c_void, l: usize, p: c_int, f: c_int, d: c_int, o: i64) -> *mut c_void;
    fn mprotect(a: *mut c_void, l: usize, p: c_int) -> c_int;
}

/// Pages in use, one bit each, and how far the range is backed.
struct Arena {
    base: usize,
    table: usize,
    entry: usize,
    backed: usize,
    used: Vec<u64>,
}

static ARENA: Mutex<Option<Arena>> = Mutex::new(None);

fn reserve(bytes: usize) -> usize {
    // SAFETY: a new anonymous mapping.
    let p = unsafe { mmap(core::ptr::null_mut(), bytes, PROT_NONE, MAP_PRIVATE | MAP_ANON, -1, 0) };
    assert_ne!(p as isize, -1, "bsdkern: no address range");
    p as usize
}

fn back(addr: usize, bytes: usize) -> bool {
    // SAFETY: inside a reservation of ours.
    unsafe { mprotect(addr as *mut c_void, bytes, PROT_RW) == 0 }
}

impl Arena {
    fn is_used(&self, page: usize) -> bool {
        self.used[page / 64] & 1 << (page % 64) != 0
    }

    fn mark(&mut self, first: usize, pages: usize, on: bool) {
        for p in first..first + pages {
            if on {
                self.used[p / 64] |= 1 << (p % 64);
            } else {
                self.used[p / 64] &= !(1 << (p % 64));
            }
        }
    }

    /// Backs one more chunk, and the `vm_page`s for it.
    fn grow(&mut self) -> bool {
        if self.backed + CHUNK_PAGES > RESERVED_PAGES {
            return false;
        }
        let from = self.table + self.backed * self.entry;
        let to = self.table + (self.backed + CHUNK_PAGES) * self.entry;
        let (from, to) = (from & !(PAGE - 1), (to + PAGE - 1) & !(PAGE - 1));
        if !back(self.base + self.backed * PAGE, CHUNK_PAGES * PAGE) || !back(from, to - from) {
            return false;
        }
        self.backed += CHUNK_PAGES;
        self.used.resize(self.backed.div_ceil(64), 0);
        true
    }

    fn find(&self, pages: usize, align: usize) -> Option<usize> {
        let mut at = 0;
        while at + pages <= self.backed {
            match (at..at + pages).rev().find(|&p| self.is_used(p)) {
                None => return Some(at),
                Some(used) => at = (used + 1).next_multiple_of(align),
            }
        }
        None
    }
}

/// Reserves the range, for `vm_page`s of `entry` bytes.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_mem_init(entry: usize) {
    let arena = Arena {
        base: reserve(RESERVED_PAGES * PAGE),
        table: reserve(RESERVED_PAGES * entry),
        entry,
        backed: 0,
        used: Vec::new(),
    };
    *ARENA.lock().unwrap() = Some(arena);
}

/// The range's start and its `vm_page`s, which don't change.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_mem_base(table: *mut usize) -> usize {
    let guard = ARENA.lock().unwrap();
    let a = guard.as_ref().expect("bsdkern: no arena");
    // SAFETY: the caller's pointer.
    unsafe { table.write(a.table) };
    a.base
}

/// `pages` pages at a multiple of `align` pages; 0 if there is no memory.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_pages_alloc(pages: usize, align: usize) -> usize {
    let mut guard = ARENA.lock().unwrap();
    let a = guard.as_mut().expect("bsdkern: no arena");
    loop {
        if let Some(at) = a.find(pages, align.max(1)) {
            a.mark(at, pages, true);
            return a.base + at * PAGE;
        }
        if !a.grow() {
            return 0;
        }
    }
}

/// Gives back what `bsdkern_pages_alloc()` gave.
#[unsafe(no_mangle)]
pub extern "C" fn bsdkern_pages_free(addr: usize, pages: usize) {
    let mut guard = ARENA.lock().unwrap();
    let a = guard.as_mut().expect("bsdkern: no arena");
    a.mark((addr - a.base) / PAGE, pages, false);
}

/// How many pages are backed: for tests, and a `sysctl` later.
pub fn backed() -> usize {
    ARENA.lock().unwrap().as_ref().map_or(0, |a| a.backed)
}

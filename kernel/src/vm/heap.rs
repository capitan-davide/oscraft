//! Kernel heap: the global allocator behind `alloc` (`Box`, `Vec`, ...).
//!
//! Small blocks come from per-size-class free lists (16 bytes to 2 KiB,
//! powers of two), refilled a page at a time; larger blocks are whole pages
//! from `kmem`, whose frames go back to `pmem` on `dealloc` (and their
//! kernel addresses to `kmem`). Small blocks' pages stay the heap's.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr::null_mut;

use crate::arch::mmu::Prot;
use crate::sync::IrqSpinLock;
use crate::vm::kmem;
use crate::vm::pmem::PAGE_SIZE;

const MIN_CLASS: usize = 4; // 16 bytes
const CLASSES: usize = 8; // 16 .. 2048 bytes

struct FreeBlock {
    next: *mut FreeBlock,
}

struct Heap {
    free: [*mut FreeBlock; CLASSES],
}

// SAFETY: the free lists are only touched under the heap lock.
unsafe impl Send for Heap {}

pub struct KernelHeap {
    heap: IrqSpinLock<Heap>,
}

#[global_allocator]
static HEAP: KernelHeap = KernelHeap {
    heap: IrqSpinLock::new(Heap {
        free: [null_mut(); CLASSES],
    }),
};

/// Size class for a layout, or `None` for page-sized allocations.
fn class(layout: Layout) -> Option<usize> {
    let size = layout.size().max(layout.align()).max(1 << MIN_CLASS);
    let class = size.next_power_of_two().trailing_zeros() as usize - MIN_CLASS;
    (class < CLASSES).then_some(class)
}

impl Heap {
    fn refill(&mut self, class: usize) -> bool {
        let Some(page) = kmem::map_new(1, Prot::Data) else {
            return false;
        };
        let size = 1usize << (class + MIN_CLASS);
        for off in (0..PAGE_SIZE as usize).step_by(size).rev() {
            let block = (page as usize + off) as *mut FreeBlock;
            // SAFETY: the block lies in the fresh page, aligned to its size.
            unsafe {
                block.write(FreeBlock {
                    next: self.free[class],
                })
            };
            self.free[class] = block;
        }
        true
    }
}

// SAFETY: blocks are handed out once until freed; sizes and alignments
// are rounded up to the power-of-two class, whose blocks are aligned to
// their size.
unsafe impl GlobalAlloc for KernelHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(c) = class(layout) else {
            let pages = (layout.size() as u64).div_ceil(PAGE_SIZE);
            return kmem::map_new(pages, Prot::Data).map_or(null_mut(), |va| va as *mut u8);
        };
        let mut h = self.heap.lock();
        if h.free[c].is_null() && !h.refill(c) {
            return null_mut();
        }
        let block = h.free[c];
        // SAFETY: non-null free-list entries are valid blocks.
        h.free[c] = unsafe { (*block).next };
        block.cast()
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let Some(c) = class(layout) else {
            let pages = (layout.size() as u64).div_ceil(PAGE_SIZE);
            kmem::unmap_new(ptr as u64, pages);
            return;
        };
        let mut h = self.heap.lock();
        let block = ptr.cast::<FreeBlock>();
        // SAFETY: `ptr` came from `alloc` with the same layout, so it is a
        // block of class `c` that nobody uses any more.
        unsafe { block.write(FreeBlock { next: h.free[c] }) };
        h.free[c] = block;
    }
}

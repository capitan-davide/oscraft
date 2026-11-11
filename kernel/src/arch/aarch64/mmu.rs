//! The kernel address space.
//!
//! The kernel keeps the translation tables startup built for it, as QNX's
//! procnto does. Their layout is part of the QNX 8.0 interface
//! (SDP `<aarch64/mmu.h>`, 4 KiB granule):
//!
//! - TTBR1 covers the top 2^40 bytes with a two-entry L0 table per CPU:
//!   entry 0 is the message-passing window, entry 1 the kernel space at
//!   `KERN_BASE`, one L1 table shared by all CPUs.
//! - The L1 table maps itself at index 0 (recursive mapping), so every L3
//!   table appears in `[KERN_BASE, KERN_BASE + 1 GiB)` and each translation
//!   entry has a fixed virtual address (`l3_entry`).
//! - L1 index 1 is startup's L2 table, whose entry 0 points back at the L1
//!   table, so the L2 tables appear at `L2_BASE` (`l2_entry`). Its other
//!   entries map startup's data (system page, callouts) from
//!   `KERN_BASE + 1 GiB + 2 MiB` and the bootstrap executables (this kernel)
//!   from `BOOTPGM_BASE`.
//!
//! Because of the recursion, a table descriptor is also read as a page
//! descriptor (through the windows), so table descriptors carry page
//! attributes too (access flag, memory type), exactly as startup writes them.
//! Page frames are zeroed through a scratch mapping before they become
//! translation tables, so the hardware never walks stale data.

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::vm::pmem::{self, PAGE_SIZE};

pub const KERN_BASE: u64 = 0xffff_ff80_0000_0000;
const L2_BASE: u64 = KERN_BASE + (1 << 30);
pub const BOOTPGM_BASE: u64 = KERN_BASE + (1 << 30) + (256 << 21);
/// First kernel address startup leaves unused.
pub const UNUSED_BASE: u64 = BOOTPGM_BASE + (256 << 21);
/// RAM is mapped here at its physical address plus this (`map_block`,
/// `kmem::init_linear`), up to physical address `LINEAR_MAX`.
pub const LINEAR_BASE: u64 = KERN_BASE + (256 << 30);
pub const LINEAR_MAX: u64 = 128 << 30;
/// A level-2 block.
pub const BLOCK_SIZE: u64 = 1 << 21;

/// The page used to zero frames before they become tables, once chosen.
static SCRATCH: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// A page next to the kernel image that startup left unmapped, in one of
/// the image's 2 MiB blocks (which startup maps with pages), used to zero
/// frames before they become tables: the last page of the block that holds
/// the image's end, or, when the image reaches into that page, the page
/// just below the image.
fn scratch_va() -> u64 {
    match SCRATCH.load(core::sync::atomic::Ordering::Relaxed) {
        0 => {
            let va = pick_scratch().expect("no free page next to the kernel image");
            SCRATCH.store(va, core::sync::atomic::Ordering::Relaxed);
            va
        }
        va => va,
    }
}

/// The guard page below CPU 0's boot stack (`kernel.ld`), unmapped by
/// `init`.
pub fn boot_stack_guard() -> u64 {
    unsafe extern "C" {
        static __stack_guard: u8;
    }
    &raw const __stack_guard as u64
}

fn pick_scratch() -> Option<u64> {
    unsafe extern "C" {
        static __kernel_start: u8;
        static __kernel_end: u8;
    }
    let start = &raw const __kernel_start as u64 & !(PAGE_SIZE - 1);
    let end = &raw const __kernel_end as u64;
    [end.next_multiple_of(BLOCK_SIZE) - PAGE_SIZE, start - PAGE_SIZE]
        .into_iter()
        .filter(|&va| va >= end || va < start)
        .find(|&va| {
            read_entry(l2_entry(va)) & DESC_TABLE_OR_PAGE == DESC_TABLE_OR_PAGE
                && read_entry(l3_entry(va)) & DESC_VALID == 0
        })
}

// Descriptor bits (<aarch64/mmu.h>).
const DESC_VALID: u64 = 1 << 0;
const DESC_TABLE_OR_PAGE: u64 = 3;
const PTE_ATTR_NORMAL: u64 = 7 << 2; // MAIR index 7, write-back write-allocate
const PTE_ATTR_DEVICE: u64 = 1 << 2; // MAIR index 1, Device-nGnRE
const PTE_AP_RO: u64 = 2 << 6;
const PTE_SH_INNER: u64 = 3 << 8;
const PTE_AF: u64 = 1 << 10;
const PTE_PXN: u64 = 1 << 53;
const PTE_UXN: u64 = 1 << 54;
const PTE_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

const TCR_EPD0: u64 = 1 << 7;

/// MAIR_EL1 attributes the kernel relies on (startup programs MAIR_EL1).
const MAIR_DEVICE_NGNRE: u64 = 0x04;
const MAIR_NORMAL_WBWA: u64 = 0xff;
const MAIR_NORMAL_NC: u64 = 0x44;

/// Mapping protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prot {
    /// Kernel read/write data.
    Data,
    /// Kernel code: read-only, executable at EL1.
    Code,
    /// Kernel read-only data.
    ReadOnly,
    /// A device's registers (Device-nGnRE), read/write, not executable.
    Device,
}

impl Prot {
    fn pte_bits(self) -> u64 {
        let normal = PTE_ATTR_NORMAL | PTE_SH_INNER | PTE_AF;
        match self {
            Prot::Data => normal | PTE_PXN | PTE_UXN,
            Prot::Code => normal | PTE_AP_RO | PTE_UXN,
            Prot::ReadOnly => normal | PTE_AP_RO | PTE_PXN | PTE_UXN,
            Prot::Device => PTE_ATTR_DEVICE | PTE_AF | PTE_PXN | PTE_UXN,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    /// Not a mappable kernel address (outside kernel space or in the
    /// recursive table window).
    BadAddress,
    AlreadyMapped,
    OutOfMemory,
}

fn l1_entry(va: u64) -> *mut u64 {
    (KERN_BASE + ((va - KERN_BASE) >> 30) * 8) as *mut u64
}

fn l2_entry(va: u64) -> *mut u64 {
    (L2_BASE + ((va - KERN_BASE) >> 21) * 8) as *mut u64
}

fn l3_entry(va: u64) -> *mut u64 {
    (KERN_BASE + ((va - KERN_BASE) >> 12) * 8) as *mut u64
}

fn read_entry(e: *mut u64) -> u64 {
    // SAFETY: callers only pass entries whose table is mapped.
    unsafe { e.read_volatile() }
}

#[expect(
    clippy::multiple_unsafe_ops_per_block,
    reason = "the entry write is sound only once the barrier orders it before the table walker"
)]
fn write_entry(e: *mut u64, v: u64) {
    // SAFETY: as for read_entry; ordering against the table walker below.
    unsafe {
        e.write_volatile(v);
        asm!("dsb ishst", "isb", options(nostack, preserves_flags));
    }
}

fn tlb_invalidate(va: u64) {
    // SAFETY: TLB maintenance for a kernel page.
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vaae1is, {}",
            "dsb ish",
            "isb",
            in(reg) va >> 12,
            options(nostack, preserves_flags)
        );
    }
}

fn sysreg_tcr() -> u64 {
    let v;
    // SAFETY: reading TCR_EL1 has no side effects.
    unsafe { asm!("mrs {}, tcr_el1", out(reg) v, options(nomem, nostack)) };
    v
}

fn sysreg_mair() -> u64 {
    let v;
    // SAFETY: reading MAIR_EL1 has no side effects.
    unsafe { asm!("mrs {}, mair_el1", out(reg) v, options(nomem, nostack)) };
    v
}

/// The CPU's physical address size in bits (`ID_AA64MMFR0_EL1.PARange`).
pub fn pa_bits() -> u32 {
    let v: u64;
    // SAFETY: reading ID_AA64MMFR0_EL1 has no side effects.
    unsafe { asm!("mrs {}, id_aa64mmfr0_el1", out(reg) v, options(nomem, nostack)) };
    match v & 0xf {
        0 => 32,
        1 => 36,
        2 => 40,
        3 => 42,
        4 => 44,
        5 => 48,
        _ => 52,
    }
}

/// What `init` found, for the boot report.
#[derive(Debug)]
pub struct Info {
    pub tcr: u64,
    pub mair: u64,
    pub t1sz: u64,
}

/// Check startup's MMU setup and switch off its identity map (TTBR0),
/// which only served the jump to the kernel.
pub fn init() -> Info {
    let mair = sysreg_mair();
    let attr = |i: u64| (mair >> (8 * i)) & 0xff;
    assert!(
        attr(1) == MAIR_DEVICE_NGNRE && attr(4) == MAIR_NORMAL_NC && attr(7) == MAIR_NORMAL_WBWA,
        "unexpected MAIR_EL1 {mair:#x}"
    );
    // The scratch page: found free now (the image mapped with pages).
    scratch_va();
    // The page below CPU 0's boot stack becomes its guard.
    unmap_page(boot_stack_guard());

    let tcr = init_cpu();
    Info {
        tcr,
        mair,
        t1sz: (tcr >> 16) & 0x3f,
    }
}

/// Per-CPU part of `init`: stop translation table walks through TTBR0 on
/// this CPU. Returns the new TCR_EL1.
pub fn init_cpu() -> u64 {
    note_cache_lines();
    let tcr = sysreg_tcr() | TCR_EPD0;
    // SAFETY: nothing the kernel uses is mapped through TTBR0.
    unsafe {
        asm!(
            "msr tcr_el1, {}",
            "isb",
            "tlbi vmalle1",
            "dsb nsh",
            "isb",
            in(reg) tcr,
            options(nostack, preserves_flags)
        );
    }
    tcr
}

/// The kernel's translation tables change only under this lock.
/// `ensure_table` is read, allocate, write, and `zero_frame` has one
/// scratch slot for every CPU: two CPUs mapping at once (thread stacks,
/// large heap blocks) could otherwise both install a table for the same
/// entry, leaving one CPU's pages in an orphaned table, or re-point the
/// scratch page under each other's zeroing. Reads (`kernel_frame`, the
/// walker) take no lock: an entry is one aligned 64-bit word.
static TABLES: crate::sync::IrqSpinLock<()> = crate::sync::IrqSpinLock::new(());

/// Zero a page frame through the scratch mapping (under `TABLES`).
fn zero_frame(pa: u64) {
    let scratch = scratch_va();
    let slot = l3_entry(scratch);
    write_entry(slot, pa | Prot::Data.pte_bits() | DESC_TABLE_OR_PAGE);
    // SAFETY: `scratch` now maps exactly this frame.
    unsafe { core::ptr::write_bytes(scratch as *mut u8, 0, PAGE_SIZE as usize) };
    write_entry(slot, 0);
    tlb_invalidate(scratch);
}

/// Make `entry` point to a translation table, allocating one if needed
/// (under `TABLES`).
fn ensure_table(entry: *mut u64) -> Result<(), MapError> {
    let e = read_entry(entry);
    if e & DESC_VALID != 0 {
        return if e & DESC_TABLE_OR_PAGE == DESC_TABLE_OR_PAGE {
            Ok(())
        } else {
            Err(MapError::AlreadyMapped)
        };
    }
    let pa = pmem::alloc_page().ok_or(MapError::OutOfMemory)?;
    zero_frame(pa);
    // Page attributes as well: the recursive windows map this entry as a page.
    write_entry(entry, pa | Prot::Data.pte_bits() | DESC_TABLE_OR_PAGE);
    Ok(())
}

fn check(va: u64) -> Result<(), MapError> {
    if va < KERN_BASE + (1 << 30) || !va.is_multiple_of(PAGE_SIZE) {
        return Err(MapError::BadAddress);
    }
    Ok(())
}

/// Map one page of the kernel address space.
pub fn map_page(va: u64, pa: u64, prot: Prot) -> Result<(), MapError> {
    check(va)?;
    let _tables = TABLES.lock();
    ensure_table(l1_entry(va))?;
    ensure_table(l2_entry(va))?;
    let e = l3_entry(va);
    if read_entry(e) & DESC_VALID != 0 {
        return Err(MapError::AlreadyMapped);
    }
    write_entry(
        e,
        (pa & PTE_ADDR_MASK) | prot.pte_bits() | DESC_TABLE_OR_PAGE,
    );
    Ok(())
}

/// Map a 2 MiB block of the kernel address space (a level-2 block
/// descriptor), for the linear map of RAM.
pub fn map_block(va: u64, pa: u64, prot: Prot) -> Result<(), MapError> {
    check(va)?;
    if !va.is_multiple_of(BLOCK_SIZE) || !pa.is_multiple_of(BLOCK_SIZE) {
        return Err(MapError::BadAddress);
    }
    let _tables = TABLES.lock();
    ensure_table(l1_entry(va))?;
    let e = l2_entry(va);
    if read_entry(e) & DESC_VALID != 0 {
        return Err(MapError::AlreadyMapped);
    }
    // Block descriptor: valid, not a table. Its level-3 view in the
    // recursive windows is invalid, so nothing maps pages inside it.
    write_entry(e, (pa & PTE_ADDR_MASK) | prot.pte_bits() | DESC_VALID);
    Ok(())
}

/// Point the window page `va`, which only this CPU uses, at frame `pa`
/// (whatever it mapped before): only this CPU's TLB can hold the old
/// translation, so only it is invalidated.
pub fn set_window(va: u64, pa: u64, prot: Prot) -> Result<(), MapError> {
    check(va)?;
    {
        let _tables = TABLES.lock();
        ensure_table(l1_entry(va))?;
        ensure_table(l2_entry(va))?;
    }
    let e = l3_entry(va);
    // Break before make (Arm ARM, "Using break-before-make when updating
    // translation table entries"): the entry usually maps another frame
    // already, and changing a valid entry's output address in place lets
    // the TLB hold both translations at once (a TLB conflict abort, or
    // reads from either frame). So: invalid first, the old translation
    // out of this CPU's TLB, then the new entry.
    #[expect(
        clippy::multiple_unsafe_ops_per_block,
        reason = "break-before-make: the writes are sound only in this order, with these barriers"
    )]
    // SAFETY: the entry's table is mapped (ensured above); the barriers
    // order each write before the walker and the local invalidation.
    unsafe {
        e.write_volatile(0);
        asm!(
            "dsb ishst",
            "tlbi vale1, {}",
            "dsb nsh",
            "isb",
            in(reg) va >> 12,
            options(nostack, preserves_flags)
        );
        e.write_volatile((pa & PTE_ADDR_MASK) | prot.pte_bits() | DESC_TABLE_OR_PAGE);
        asm!("dsb ishst", "isb", options(nostack, preserves_flags));
    }
    Ok(())
}

/// Make sure the tables that hold the page entry for kernel address `va`
/// exist (for code that maps pages there itself, e.g. QNX's kdumper).
pub fn ensure_tables(va: u64) -> Result<(), MapError> {
    check(va)?;
    let _tables = TABLES.lock();
    ensure_table(l1_entry(va))?;
    ensure_table(l2_entry(va))
}

/// Is there an L3 table for `va`: are its L1 and L2 entries tables (not
/// blocks, not empty)? Only then may its L3 entry be read through the
/// recursive window.
fn l3_reachable(va: u64) -> bool {
    let table = |e: *mut u64| read_entry(e) & DESC_TABLE_OR_PAGE == DESC_TABLE_OR_PAGE;
    table(l1_entry(va)) && table(l2_entry(va))
}

/// The frame a kernel page (`va` page-aligned, above the recursive
/// windows) maps, if it is mapped with a page (not a block).
pub fn kernel_frame(va: u64) -> Option<u64> {
    check(va).ok()?;
    if !l3_reachable(va) {
        return None;
    }
    let e = read_entry(l3_entry(va));
    (e & DESC_TABLE_OR_PAGE == DESC_TABLE_OR_PAGE).then_some(e & PTE_ADDR_MASK)
}

/// The frames of the kernel's translation tables, one per call, from
/// `*cursor` (0 to start; `None` at the end): this CPU's L0 table, the L1
/// table, then each L2 table and the L3 tables under it. For a crash dump,
/// so that kernel addresses translate in it.
pub fn next_table_frame(cursor: &mut u64) -> Option<u64> {
    const N: u64 = 512;
    let table = |e: u64| (e & DESC_TABLE_OR_PAGE == DESC_TABLE_OR_PAGE).then_some(e & PTE_ADDR_MASK);
    loop {
        let c = *cursor;
        *cursor += 1;
        match c {
            0 => {
                let ttbr1: u64;
                // SAFETY: reads a system register.
                unsafe { asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack)) };
                return Some(ttbr1 & PTE_ADDR_MASK);
            }
            // L1 index 0 maps the L1 table itself.
            1 => return table(read_entry(l1_entry(KERN_BASE))),
            _ => {}
        }
        // Per L1 entry: its L2 table, then the L3 tables of the L2 entries.
        let (i, k) = ((c - 2) / (N + 1), (c - 2) % (N + 1));
        if i >= N {
            *cursor = c;
            return None;
        }
        let va = KERN_BASE + (i << 30);
        let l2 = if i == 0 { None } else { table(read_entry(l1_entry(va))) };
        if l2.is_none() {
            *cursor = 2 + (i + 1) * (N + 1);
            continue;
        }
        if k == 0 {
            return l2;
        }
        // Startup's L2 table's entry 0 is the L1 table again.
        if i == 1 && k == 1 {
            continue;
        }
        if let Some(l3) = table(read_entry(l2_entry(va + ((k - 1) << 21)))) {
            return Some(l3);
        }
    }
}

/// Remove a page mapping; returns the frame it mapped.
pub fn unmap_page(va: u64) -> Option<u64> {
    check(va).ok()?;
    let _tables = TABLES.lock();
    if !l3_reachable(va) || read_entry(l3_entry(va)) & DESC_VALID == 0 {
        return None;
    }
    let old = read_entry(l3_entry(va));
    write_entry(l3_entry(va), 0);
    tlb_invalidate(va);
    Some(old & PTE_ADDR_MASK)
}

/// Remove the page mappings of `pages` pages from `va` (pages not mapped
/// are skipped) and pass each frame they mapped to `free`, once no CPU's
/// TLB has it any more. In batches of 64 pages: the entries cleared, then
/// one barrier, their invalidations, and one barrier for the batch, not
/// one per page (review CORE-21). Allocates nothing: the heap frees
/// through here.
pub fn unmap_pages(va: u64, pages: u64, mut free: impl FnMut(u64)) {
    const BATCH: u64 = 64;
    let mut done = 0;
    while done < pages {
        let n = (pages - done).min(BATCH);
        let base = va + done * PAGE_SIZE;
        // The frames cleared, with bit 0 set (a frame may be 0).
        let mut frames = [0u64; BATCH as usize];
        {
            let _tables = TABLES.lock();
            for (i, f) in frames.iter_mut().enumerate().take(n as usize) {
                let a = base + i as u64 * PAGE_SIZE;
                if check(a).is_err() || !l3_reachable(a) {
                    continue;
                }
                let e = read_entry(l3_entry(a));
                if e & DESC_VALID != 0 {
                    write_entry(l3_entry(a), 0);
                    *f = (e & PTE_ADDR_MASK) | 1;
                }
            }
            // SAFETY: a barrier only: the cleared entries are seen before
            // the invalidations.
            unsafe { asm!("dsb ishst", options(nostack, preserves_flags)) };
            for (i, _) in frames.iter().enumerate().take(n as usize).filter(|(_, f)| **f != 0) {
                let a = base + i as u64 * PAGE_SIZE;
                // SAFETY: TLB maintenance for a kernel page.
                unsafe { asm!("tlbi vaae1is, {}", in(reg) a >> 12, options(nostack, preserves_flags)) };
            }
            // SAFETY: barriers only: every CPU has dropped the entries.
            unsafe { asm!("dsb ish", "isb", options(nostack, preserves_flags)) };
        }
        for f in frames.iter().take(n as usize).filter(|f| **f != 0) {
            free(f & !1);
        }
        done += n;
    }
}

/// Change the protection of a mapped page.
pub fn protect(va: u64, prot: Prot) -> Result<(), MapError> {
    check(va)?;
    let _tables = TABLES.lock();
    // Under a block, or no table, there is no L3 entry to read: its window
    // address would itself fault (review CORE-15).
    if !l3_reachable(va) {
        return Err(MapError::BadAddress);
    }
    let e = l3_entry(va);
    let old = read_entry(e);
    if old & DESC_VALID == 0 {
        return Err(MapError::BadAddress);
    }
    write_entry(e, 0);
    tlb_invalidate(va);
    write_entry(
        e,
        (old & PTE_ADDR_MASK) | prot.pte_bits() | DESC_TABLE_OR_PAGE,
    );
    Ok(())
}

/// Make instructions written to `[va, va + len)` visible to instruction
/// fetch: clean the data cache and invalidate the instruction cache to the
/// point of unification.
pub fn sync_icache(va: u64, len: u64) {
    let (dline, iline) = cache_lines();
    let mut a = va & !(dline - 1);
    while a < va + len {
        // SAFETY: cache maintenance on mapped kernel memory.
        unsafe { asm!("dc cvau, {}", in(reg) a, options(nostack, preserves_flags)) };
        a += dline;
    }
    // SAFETY: a barrier only: the cleaning completes before the
    // invalidation.
    unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
    let mut a = va & !(iline - 1);
    while a < va + len {
        // SAFETY: cache maintenance on mapped kernel memory.
        unsafe { asm!("ic ivau, {}", in(reg) a, options(nostack, preserves_flags)) };
        a += iline;
    }
    // SAFETY: barriers only: the maintenance above completes.
    unsafe { asm!("dsb ish", "isb", options(nostack, preserves_flags)) };
}

/// The smallest data and instruction cache lines of the CPUs started so
/// far, in bytes (`note_cache_lines`): maintenance by address steps by
/// them, so that no line of any CPU is skipped (review CORE-11).
static DLINE: AtomicU64 = AtomicU64::new(u64::MAX);
static ILINE: AtomicU64 = AtomicU64::new(u64::MAX);

/// Record this CPU's cache line sizes, from `CTR_EL0` (`DminLine` and
/// `IminLine`: log2 of the line in 4-byte words).
fn note_cache_lines() {
    let ctr: u64;
    // SAFETY: reading CTR_EL0 has no side effects.
    unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags)) };
    DLINE.fetch_min(4 << ((ctr >> 16) & 0xf), Ordering::Relaxed);
    ILINE.fetch_min(4 << (ctr & 0xf), Ordering::Relaxed);
}

/// The data and instruction cache line sizes to step by.
fn cache_lines() -> (u64, u64) {
    if DLINE.load(Ordering::Relaxed) == u64::MAX {
        note_cache_lines(); // before `init`
    }
    (DLINE.load(Ordering::Relaxed), ILINE.load(Ordering::Relaxed))
}

/// Write `[va, va + len)` back from the data cache to memory and drop it
/// from the cache (to the point of coherency): before memory is used
/// through an uncached mapping (a device's DMA buffers), so that no dirty
/// line is written back over it later.
pub fn clean_inval_dcache(va: u64, len: u64) {
    let line = cache_lines().0;
    let mut a = va & !(line - 1);
    while a < va + len {
        // SAFETY: cache maintenance on mapped kernel memory.
        unsafe { asm!("dc civac, {}", in(reg) a, options(nostack, preserves_flags)) };
        a += line;
    }
    // SAFETY: barriers only: the maintenance above completes.
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) };
}

/// Write `[va, va + len)` back from the data cache to memory (to the point
/// of coherency), keeping it cached: before a device reads it.
pub fn clean_dcache(va: u64, len: u64) {
    let line = cache_lines().0;
    let mut a = va & !(line - 1);
    while a < va + len {
        // SAFETY: cache maintenance on mapped kernel memory.
        unsafe { asm!("dc cvac, {}", in(reg) a, options(nostack, preserves_flags)) };
        a += line;
    }
    // SAFETY: barriers only: the maintenance above completes.
    unsafe { asm!("dsb sy", options(nostack, preserves_flags)) };
}

/// Physical address of a mapped kernel address, from the MMU itself.
pub fn translate(va: u64) -> Option<u64> {
    let par: u64;
    // SAFETY: address translation instruction; no memory access.
    unsafe {
        asm!("at s1e1r, {}", "isb", "mrs {}, par_el1", in(reg) va, out(reg) par, options(nostack));
    }
    (par & 1 == 0).then_some((par & PTE_ADDR_MASK) | (va & (PAGE_SIZE - 1)))
}

/// May EL1 write `va`? The MMU's answer (`AT S1E1W`).
pub fn writable(va: u64) -> bool {
    let par: u64;
    let daif = crate::arch::irq_save();
    // SAFETY: address translation instruction; no memory access.
    unsafe {
        asm!("at s1e1w, {}", "isb", "mrs {}, par_el1", in(reg) va, out(reg) par, options(nostack));
    }
    crate::arch::irq_restore(daif);
    par & 1 == 0
}

/// The physical range of the kernel's code and read-only data, if the
/// image is contiguous there (as startup loads it): what the linear map
/// must not alias writable (review CORE-23).
pub fn kernel_ro_phys() -> Option<(u64, u64)> {
    unsafe extern "C" {
        static __kernel_start: u8;
        static __ro_end: u8;
    }
    let start = &raw const __kernel_start as u64 & !(PAGE_SIZE - 1);
    let end = &raw const __ro_end as u64;
    let pa = translate(start)? & !(PAGE_SIZE - 1);
    let contiguous = (start..end)
        .step_by(PAGE_SIZE as usize)
        .all(|va| translate(va).map(|p| p & !(PAGE_SIZE - 1)) == Some(pa + (va - start)));
    contiguous.then_some((pa, pa + (end - start)))
}

/// Map a fresh page where startup mapped nothing (so new L2 and L3 tables
/// are needed), check it through the MMU, use it and unmap it.
pub fn selftest() -> Result<(u64, u64), &'static str> {
    let va = UNUSED_BASE;
    let pa = pmem::alloc_page().ok_or("out of memory")?;
    map_page(va, pa, Prot::Data).map_err(|_| "map_page failed")?;
    let ok = translate(va) == Some(pa);
    let p = va as *mut u64;
    // SAFETY: va was just mapped read/write to a frame we own.
    unsafe { p.write_volatile(0x05c4_af7d_eadb_eef0) };
    // SAFETY: as above.
    let readback = unsafe { p.read_volatile() };
    let unmapped = unmap_page(va) == Some(pa) && translate(va).is_none();
    match (ok, readback == 0x05c4_af7d_eadb_eef0, unmapped) {
        (false, _, _) => Err("translation does not match"),
        (_, false, _) => Err("read back wrong value"),
        (_, _, false) => Err("unmap failed"),
        _ => Ok((va, pa)),
    }
}

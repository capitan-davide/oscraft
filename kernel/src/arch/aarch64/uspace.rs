//! User address spaces (TTBR0).
//!
//! Each process has its own translation tables for `[0, USER_END)` (TTBR0,
//! T0SZ = 25 as startup set it: 39 bits, three levels from L1) and an ASID,
//! so switching processes needs no TLB flush. There is no direct map of
//! physical memory, so every table page is also mapped at a kernel address,
//! recorded per address space (`kva`).
//!
//! The Cortex-A72 is ARMv8.0 (no PAN), so the kernel can reach the current
//! process's pages directly through TTBR0.

use alloc::collections::BTreeMap;
use core::arch::asm;
use core::sync::atomic::{AtomicU32, Ordering};

use super::mmu::MapError;
use crate::sync::IrqSpinLock;
use crate::vm::kmem;
use crate::vm::pmem::{self, PAGE_SIZE};

/// End of user space (`AARCH64_USER_END` in `<aarch64/mmu.h>`).
pub const USER_END: u64 = 1 << 39;

const DESC_VALID: u64 = 1;
const DESC_TABLE_OR_PAGE: u64 = 3;
const PTE_ATTR_NORMAL: u64 = 7 << 2;
const PTE_ATTR_MASK: u64 = 7 << 2;
/// MAIR index 1: Device-nGnRE (`arch::mmu` checks startup's MAIR).
const PTE_ATTR_DEVICE: u64 = 1 << 2;
/// MAIR index 4: Normal Non-cacheable (also checked by `arch::mmu`).
const PTE_ATTR_NORMAL_NC: u64 = 4 << 2;
const PTE_AP_EL0_RW: u64 = 1 << 6;
const PTE_AP_EL0_RO: u64 = 3 << 6;
const PTE_AP_MASK: u64 = 3 << 6;
const PTE_SH_INNER: u64 = 3 << 8;
const PTE_AF: u64 = 1 << 10;
const PTE_NG: u64 = 1 << 11;
const PTE_PXN: u64 = 1 << 53;
const PTE_UXN: u64 = 1 << 54;
const PTE_ADDR_MASK: u64 = 0x0000_ffff_ffff_f000;
const TCR_EPD0: u64 = 1 << 7;

/// ASID 0 is process 1's (empty) user space. Released ASIDs are reused
/// first (a counter alone would wrap and alias live address spaces). 16
/// bits wide if the CPU has them and startup chose them (`TCR_EL1.AS`),
/// else 8 (`asid_max`).
/// The next ASID never handed out (0 is process 1's, the boot address
/// space); freed ones (`FREE_ASIDS`) are reused first. 32 bits, so that
/// `u16::MAX` itself can be handed out and the counter cannot wrap onto 0.
static NEXT_ASID: AtomicU32 = AtomicU32::new(0);
static FREE_ASIDS: IrqSpinLock<alloc::vec::Vec<u16>> = IrqSpinLock::new(alloc::vec::Vec::new());

/// The highest ASID the TLB tells apart: 65535 with 16-bit ASIDs
/// (`ID_AA64MMFR0_EL1.ASIDBits` 0b0010 and `TCR_EL1.AS` set), else 255.
/// Beyond it address spaces would share TLB entries (review CORE-8).
fn asid_max() -> u32 {
    let (mmfr0, tcr): (u64, u64);
    // SAFETY: reading ID registers and TCR_EL1 has no side effects.
    unsafe {
        asm!(
            "mrs {}, id_aa64mmfr0_el1",
            "mrs {}, tcr_el1",
            out(reg) mmfr0,
            out(reg) tcr,
            options(nomem, nostack, preserves_flags)
        )
    };
    const TCR_AS: u64 = 1 << 36;
    if (mmfr0 >> 4) & 0xf == 0b0010 && tcr & TCR_AS != 0 {
        u16::MAX as u32
    } else {
        u8::MAX as u32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserProt {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
}

impl UserProt {
    /// No EL0 access (PROT_NONE): the page stays mapped for the kernel.
    pub const NONE: Self = Self {
        read: false,
        write: false,
        exec: false,
    };

    fn pte_bits(self) -> u64 {
        let mut bits = if self.write {
            PTE_AP_EL0_RW
        } else if self.read || self.exec {
            PTE_AP_EL0_RO
        } else {
            0 // AP = 00: EL1 only
        };
        if !self.exec {
            bits |= PTE_UXN;
        }
        bits
    }
}

pub struct AddressSpace {
    asid: u16,
    l1: u64,
    /// Physical -> kernel virtual address of each table page.
    kva: IrqSpinLock<BTreeMap<u64, u64>>,
}

/// Give back table page `pa` (at `kva`, from `new_table`).
fn free_table(pa: u64, kva: u64) {
    if kmem::linear(pa) != Some(kva) {
        crate::arch::mmu::unmap_page(kva);
    }
    pmem::free_page(pa);
}

fn entry(table_kva: u64, index: u64) -> *mut u64 {
    (table_kva + index * 8) as *mut u64
}

/// Whether the table page at `table_kva` (one of an address space's, its
/// lock held) has no valid entry.
fn table_empty(table_kva: u64) -> bool {
    // SAFETY: a table page mapped at its kva, PAGE_SIZE bytes of u64s.
    let t = unsafe { core::slice::from_raw_parts(table_kva as *const u64, 512) };
    t.iter().all(|&d| d & DESC_VALID == 0)
}

fn tlb_invalidate(asid: u16, va: u64) {
    // SAFETY: TLB maintenance only.
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi vae1is, {}",
            "dsb ish",
            "isb",
            in(reg) ((asid as u64) << 48) | (va >> 12),
            options(nostack, preserves_flags)
        );
    }
}

/// A zeroed page frame mapped in the kernel: (physical, kernel virtual).
/// Reached through the linear map (or, for a frame outside it, mapped at
/// a kernel address of its own).
fn new_table() -> Option<(u64, u64)> {
    let pa = pmem::alloc_page()?;
    let kva = match kmem::linear(pa) {
        Some(kva) => kva,
        None => match kmem::map_frame(pa) {
            Some(kva) => kva,
            None => {
                pmem::free_page(pa);
                return None;
            }
        },
    };
    // SAFETY: freshly mapped, owned by us.
    unsafe { core::ptr::write_bytes(kva as *mut u8, 0, PAGE_SIZE as usize) };
    Some((pa, kva))
}

impl AddressSpace {
    pub fn new() -> Option<Self> {
        let asid = match FREE_ASIDS.lock().pop() {
            Some(a) => a,
            None => NEXT_ASID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |a| {
                    (a <= asid_max()).then_some(a + 1)
                })
                .ok()? as u16, // else every ASID is in use
        };
        // No table, no address space: the ASID goes back (review CORE-14:
        // failed forks under memory pressure used them up for good).
        let Some((l1, kva)) = new_table() else {
            FREE_ASIDS.lock().push(asid);
            return None;
        };
        let mut map = BTreeMap::new();
        map.insert(l1, kva);
        Some(Self {
            asid,
            l1,
            kva: IrqSpinLock::new(map),
        })
    }

    pub fn asid(&self) -> u16 {
        self.asid
    }

    /// TTBR0_EL1 value for this address space.
    pub fn ttbr0(&self) -> u64 {
        self.l1 | ((self.asid as u64) << 48)
    }

    /// The L3 entry for `va`, creating tables on the way if `create`.
    /// Run `f` on the level-3 entry of `va` (creating the tables down to
    /// it if `create`), with the table lock held: the tables can't be
    /// freed (`free_tables`) while `f` uses the entry.
    fn with_l3<R>(
        &self,
        va: u64,
        create: bool,
        f: impl FnOnce(*mut u64) -> R,
    ) -> Result<R, MapError> {
        if va >= USER_END || !va.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::BadAddress);
        }
        let mut kva = self.kva.lock();
        // None once released.
        let mut table = *kva.get(&self.l1).ok_or(MapError::BadAddress)?;
        for shift in [30, 21] {
            let e = entry(table, (va >> shift) & 511);
            // SAFETY: `table` is one of our table pages, mapped at its kva.
            let d = unsafe { e.read_volatile() };
            table = if d & DESC_VALID != 0 {
                kva[&(d & PTE_ADDR_MASK)]
            } else if create {
                let (pa, tkva) = new_table().ok_or(MapError::OutOfMemory)?;
                kva.insert(pa, tkva);
                #[expect(
                    clippy::multiple_unsafe_ops_per_block,
                    reason = "the entry write is sound only once the barrier orders it before the table walker"
                )]
                // SAFETY: as above; the new table is zeroed.
                unsafe {
                    e.write_volatile(pa | DESC_TABLE_OR_PAGE);
                    asm!("dsb ishst", options(nostack, preserves_flags));
                }
                tkva
            } else {
                return Err(MapError::BadAddress);
            };
        }
        Ok(f(entry(table, (va >> 12) & 511)))
    }

    /// Map one user page.
    pub fn map(&self, va: u64, pa: u64, prot: UserProt) -> Result<(), MapError> {
        self.map_attr(va, pa, prot, PTE_ATTR_NORMAL | PTE_SH_INNER)
    }

    /// Map one user page of device memory (Device-nGnRE: uncached, for a
    /// driver's registers; never executable).
    pub fn map_device(&self, va: u64, pa: u64, prot: UserProt) -> Result<(), MapError> {
        let prot = UserProt {
            exec: false,
            ..prot
        };
        self.map_attr(va, pa, prot, PTE_ATTR_DEVICE)
    }

    /// Map one user page of RAM uncached (Normal Non-cacheable: a DMA
    /// buffer the CPU and a device share).
    pub fn map_uncached(&self, va: u64, pa: u64, prot: UserProt) -> Result<(), MapError> {
        self.map_attr(va, pa, prot, PTE_ATTR_NORMAL_NC | PTE_SH_INNER)
    }

    fn map_attr(&self, va: u64, pa: u64, prot: UserProt, attr: u64) -> Result<(), MapError> {
        self.with_l3(va, true, |e| {
            // SAFETY: a valid L3 entry of this address space.
            if unsafe { e.read_volatile() } & DESC_VALID != 0 {
                return Err(MapError::AlreadyMapped);
            }
            let pte = (pa & PTE_ADDR_MASK) | attr | PTE_AF | PTE_NG | PTE_PXN | prot.pte_bits();
            #[expect(
                clippy::multiple_unsafe_ops_per_block,
                reason = "the entry write is sound only once the barrier orders it before the table walker"
            )]
            // SAFETY: as above.
            unsafe {
                e.write_volatile(pte | DESC_TABLE_OR_PAGE);
                asm!("dsb ishst", "isb", options(nostack, preserves_flags));
            }
            Ok(())
        })?
    }

    /// Remove a user page mapping; returns its frame.
    pub fn unmap(&self, va: u64) -> Option<u64> {
        self.with_l3(va, false, |e| {
            // SAFETY: a valid L3 entry of this address space.
            let old = unsafe { e.read_volatile() };
            if old & DESC_VALID == 0 {
                return None;
            }
            // SAFETY: as above.
            unsafe { e.write_volatile(0) };
            tlb_invalidate(self.asid, va);
            Some(old & PTE_ADDR_MASK)
        })
        .ok()?
    }

    /// Unmap the pages of `[start, end)` (page-aligned), walking the tables
    /// once and skipping the parts without tables; then, once the TLBs no
    /// longer hold them, call `f` with each page's (address, frame).
    ///
    /// A level-3 table left without a mapping is freed, and then its
    /// level-2 table if that is left empty (review PATHMEM-25b: kept until
    /// the process ended, they sat among its freed pages and cut free
    /// memory into runs of a few MiB, so a large contiguous `mmap()` failed
    /// though the memory had been freed).
    pub fn unmap_range(&self, start: u64, end: u64, mut f: impl FnMut(u64, u64)) {
        /// Up to this many pages are invalidated one by one; more, the
        /// whole ASID at once.
        const FEW: usize = 16;
        let end = end.min(USER_END);
        let mut gone = alloc::vec::Vec::new();
        // Emptied tables (physical, kernel virtual), freed after the TLB
        // invalidation: until then a walk cache may still point at them.
        let mut tables = alloc::vec::Vec::new();
        {
            let mut kva = self.kva.lock();
            let Some(&l1) = kva.get(&self.l1) else {
                return; // released already
            };
            let mut va = start;
            while va < end {
                let next_l2 = (va | ((1 << 21) - 1)) + 1;
                // SAFETY: entries of our own table pages, mapped at their kva.
                let d1 = unsafe { entry(l1, (va >> 30) & 511).read_volatile() };
                if d1 & DESC_VALID == 0 {
                    va = (va | ((1 << 30) - 1)) + 1;
                    continue;
                }
                let l2 = kva[&(d1 & PTE_ADDR_MASK)];
                // SAFETY: as above.
                let d2 = unsafe { entry(l2, (va >> 21) & 511).read_volatile() };
                if d2 & DESC_VALID == 0 {
                    va = next_l2;
                    continue;
                }
                let l3 = kva[&(d2 & PTE_ADDR_MASK)];
                let cleared = gone.len();
                while va < end.min(next_l2) {
                    let e = entry(l3, (va >> 12) & 511);
                    // SAFETY: as above.
                    let old = unsafe { e.read_volatile() };
                    if old & DESC_VALID != 0 {
                        // SAFETY: as above.
                        unsafe { e.write_volatile(0) };
                        gone.push((va, old & PTE_ADDR_MASK));
                    }
                    va += PAGE_SIZE;
                }
                if gone.len() == cleared || !table_empty(l3) {
                    continue;
                }
                // SAFETY: as above; the L3 table's last mapping is gone.
                unsafe { entry(l2, ((va - 1) >> 21) & 511).write_volatile(0) };
                tables.push((d2 & PTE_ADDR_MASK, kva.remove(&(d2 & PTE_ADDR_MASK)).unwrap_or(l3)));
                if table_empty(l2) {
                    // SAFETY: as above; the L2 table's last table is gone.
                    unsafe { entry(l1, ((va - 1) >> 30) & 511).write_volatile(0) };
                    tables.push((d1 & PTE_ADDR_MASK, kva.remove(&(d1 & PTE_ADDR_MASK)).unwrap_or(l2)));
                }
            }
        }
        if gone.len() <= FEW && tables.is_empty() {
            for &(va, _) in &gone {
                tlb_invalidate(self.asid, va);
            }
        } else {
            // SAFETY: TLB maintenance only.
            unsafe {
                asm!(
                    "dsb ishst",
                    "tlbi aside1is, {}",
                    "dsb ish",
                    "isb",
                    in(reg) (self.asid as u64) << 48,
                    options(nostack, preserves_flags)
                );
            }
        }
        for (pa, kva) in tables {
            free_table(pa, kva);
        }
        for (va, pa) in gone {
            f(va, pa);
        }
    }

    /// Free the table pages below level 1 and empty level 1, once no page
    /// is mapped any more and none of its threads runs (a process that has
    /// ended). A CPU may still have it loaded (`release`): the level-1
    /// table stays, empty, so a walk finds nothing. Its TLB entries go too.
    pub fn free_tables(&self) {
        let mut kva = self.kva.lock();
        let Some(&l1_kva) = kva.get(&self.l1) else {
            return; // released already
        };
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "freeing the lower tables is sound only after both the table is emptied and the TLB flushed"
        )]
        // SAFETY: our own level-1 table page. Emptied, and the TLB and walk
        // caches flushed, before the lower tables can be reused.
        unsafe {
            core::ptr::write_bytes(l1_kva as *mut u8, 0, PAGE_SIZE as usize);
            asm!(
                "dsb ishst",
                "tlbi aside1is, {}",
                "dsb ish",
                "isb",
                in(reg) (self.asid as u64) << 48,
                options(nostack, preserves_flags)
            );
        }
        for (&pa, &va) in kva.iter().filter(|(pa, _)| **pa != self.l1) {
            free_table(pa, va);
        }
        kva.retain(|pa, _| *pa == self.l1);
    }

    /// Give the address space back, its tables freed (after
    /// `free_tables`): its level-1 table freed and its ASID reused once no
    /// CPU has it loaded (a CPU running process 1's threads may keep it, see
    /// `loaded`). No thread may run on it any more, and it must not be used
    /// again.
    pub fn release(&self) {
        self.free_tables();
        let l1_kva = self.kva.lock().remove(&self.l1);
        if let Some(l1_kva) = l1_kva {
            if loaded() == self.ttbr0() {
                activate(crate::ker::process::KERNEL.aspace().ttbr0());
            }
            DEFERRED.lock().push((self.l1, l1_kva, self.asid));
        }
        free_deferred();
    }

    /// Change the permissions of a mapped user page; returns its frame.
    pub fn protect(&self, va: u64, prot: UserProt) -> Option<u64> {
        self.with_l3(va, false, |e| {
            // SAFETY: a valid L3 entry of this address space.
            let old = unsafe { e.read_volatile() };
            if old & DESC_VALID == 0 {
                return None;
            }
            let pte = (old & !(PTE_AP_MASK | PTE_UXN)) | prot.pte_bits();
            // SAFETY: as above.
            unsafe { e.write_volatile(pte) };
            tlb_invalidate(self.asid, va);
            Some(old & PTE_ADDR_MASK)
        })
        .ok()?
    }

    /// Make a mapped user page read-only, without the TLB invalidation:
    /// the caller invalidates the whole address space after a batch
    /// (`flush`). Returns its frame and old permissions.
    pub fn write_protect_batched(&self, va: u64) -> Option<(u64, UserProt)> {
        let (pa, prot) = self.page(va)?;
        if prot.write {
            let ro = UserProt {
                write: false,
                ..prot
            };
            self.with_l3(va, false, |e| {
                // SAFETY: a valid L3 entry of this address space.
                let old = unsafe { e.read_volatile() };
                // SAFETY: as above.
                unsafe { e.write_volatile((old & !(PTE_AP_MASK | PTE_UXN)) | ro.pte_bits()) };
            })
            .ok()?;
        }
        Some((pa, prot))
    }

    /// Invalidate every TLB entry of this address space.
    pub fn flush(&self) {
        // SAFETY: TLB maintenance only.
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi aside1is, {}",
                "dsb ish",
                "isb",
                in(reg) (self.asid as u64) << 48,
                options(nostack, preserves_flags)
            );
        }
    }

    /// Frame and permissions of a mapped user page.
    pub fn page(&self, va: u64) -> Option<(u64, UserProt)> {
        // SAFETY: a valid L3 entry of this address space.
        let pte = self
            .with_l3(va & !(PAGE_SIZE - 1), false, |e| unsafe {
                e.read_volatile()
            })
            .ok()?;
        if pte & DESC_VALID == 0 {
            return None;
        }
        let ap = pte & PTE_AP_MASK;
        let prot = UserProt {
            read: ap != 0,
            write: ap == PTE_AP_EL0_RW,
            exec: pte & PTE_UXN == 0,
        };
        Some((pte & PTE_ADDR_MASK, prot))
    }

    /// Physical address of a user address, if EL0 can access it (and
    /// write it, if asked).
    pub fn translate(&self, va: u64, write: bool) -> Option<u64> {
        self.translate_cached(va, write).map(|(pa, _)| pa)
    }

    /// `translate`, and whether the page is mapped as normal cacheable
    /// memory.
    pub fn translate_cached(&self, va: u64, write: bool) -> Option<(u64, bool)> {
        // SAFETY: a valid L3 entry of this address space.
        let pte = self
            .with_l3(va & !(PAGE_SIZE - 1), false, |e| unsafe {
                e.read_volatile()
            })
            .ok()?;
        let ap = pte & PTE_AP_MASK;
        if pte & DESC_VALID == 0 || ap == 0 || (write && ap != PTE_AP_EL0_RW) {
            return None;
        }
        Some((
            (pte & PTE_ADDR_MASK) | (va & (PAGE_SIZE - 1)),
            pte & PTE_ATTR_MASK == PTE_ATTR_NORMAL,
        ))
    }
}

/// Load an address space's tables into TTBR0 on this CPU.
pub fn activate(ttbr0: u64) {
    // SAFETY: switches only the user half of the address space.
    unsafe { asm!("msr ttbr0_el1, {}", "isb", in(reg) ttbr0, options(nostack, preserves_flags)) };
    crate::percpu::this().user_ttbr0.store(ttbr0, Ordering::Release);
}

/// The TTBR0_EL1 value loaded on this CPU. process 1's threads leave the
/// last user process's tables loaded (`sched`): switching to them and back
/// then costs no TTBR0 write.
pub fn loaded() -> u64 {
    crate::percpu::this().user_ttbr0.load(Ordering::Relaxed)
}

/// Whether some CPU has `ttbr0` loaded.
fn loaded_anywhere(ttbr0: u64) -> bool {
    (0..crate::percpu::MAX_CPUS as u32)
        .filter_map(crate::percpu::get)
        .any(|c| c.user_ttbr0.load(Ordering::Acquire) == ttbr0)
}

/// Level-1 tables (physical, kernel virtual) and ASIDs of released address
/// spaces that some CPU still had loaded: freed once none has.
static DEFERRED: IrqSpinLock<alloc::vec::Vec<(u64, u64, u16)>> =
    IrqSpinLock::new(alloc::vec::Vec::new());

/// Free the deferred level-1 tables and ASIDs no CPU has loaded any more.
fn free_deferred() {
    DEFERRED.lock().retain(|&(l1, l1_kva, asid)| {
        if loaded_anywhere(l1 | ((asid as u64) << 48)) {
            return true;
        }
        // SAFETY: no CPU walks this (empty) table any more; entries cached
        // under the ASID go before it is reused.
        unsafe {
            asm!(
                "dsb ishst",
                "tlbi aside1is, {}",
                "dsb ish",
                "isb",
                in(reg) (asid as u64) << 48,
                options(nostack, preserves_flags)
            );
        }
        free_table(l1, l1_kva);
        FREE_ASIDS.lock().push(asid);
        false
    });
}

/// Replace startup's identity map in TTBR0 by `ttbr0` and enable TTBR0
/// walks on this CPU.
pub fn init_cpu(ttbr0: u64) {
    activate(ttbr0);
    let tcr: u64;
    // SAFETY: reads a system register.
    unsafe { asm!("mrs {}, tcr_el1", out(reg) tcr, options(nomem, nostack)) };
    // SAFETY: TTBR0 now holds a valid (user) table.
    unsafe {
        asm!(
            "msr tcr_el1, {}",
            "isb",
            "tlbi vmalle1",
            "dsb nsh",
            "isb",
            in(reg) tcr & !TCR_EPD0,
            options(nostack, preserves_flags)
        );
    }
}

/// The physical address of user address `va` of the address space with
/// `ttbr0`, if this CPU has it loaded and EL0 may read it (or write it,
/// with `write`), and whether it is normal cacheable memory: the MMU's
/// translation (and its TLB), not a walk of the tables.
pub fn el0_translate(ttbr0: u64, va: u64, write: bool) -> Option<(u64, bool)> {
    let par: u64;
    // IRQs are masked so that the tables stay loaded and no handler's
    // translation replaces PAR_EL1 before it is read.
    let daif = crate::arch::irq_save();
    if loaded() != ttbr0 {
        crate::arch::irq_restore(daif);
        return None;
    }
    if write {
        // SAFETY: address translation instructions only.
        unsafe { asm!("at s1e0w, {}", "isb", "mrs {}, par_el1", in(reg) va, out(reg) par, options(nostack)) };
    } else {
        // SAFETY: as above.
        unsafe { asm!("at s1e0r, {}", "isb", "mrs {}, par_el1", in(reg) va, out(reg) par, options(nostack)) };
    }
    crate::arch::irq_restore(daif);
    // PAR_EL1.ATTR: the MAIR attribute, 0xff for write-back.
    (par & 1 == 0).then_some((
        (par & PTE_ADDR_MASK) | (va & (PAGE_SIZE - 1)),
        par >> 56 == 0xff,
    ))
}

/// EL0 read permission of `va` in the current TTBR0, checked by the MMU.
pub fn el0_readable(va: u64) -> bool {
    let par: u64;
    // IRQs masked, as in `el0_translate`: a handler's own translation
    // would replace PAR_EL1 before it is read (review CORE-20).
    let daif = crate::arch::irq_save();
    // SAFETY: address translation instruction only.
    unsafe {
        asm!("at s1e0r, {}", "isb", "mrs {}, par_el1", in(reg) va, out(reg) par, options(nostack))
    };
    crate::arch::irq_restore(daif);
    par & 1 == 0
}

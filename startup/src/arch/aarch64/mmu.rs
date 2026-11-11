//! The MMU: the page tables the kernel starts with.
//!
//! Startup itself runs with the MMU off. It builds the tables the kernel
//! expects (`<aarch64/mmu.h>`), with 4 KB pages:
//!
//! - TTBR0, the lower half (user space later), holds a 1:1 map of startup
//!   itself, so that the instruction after the MMU is turned on still
//!   runs.
//! - TTBR1, the upper half, is the kernel's: each CPU has its own level 0
//!   table of two entries, 64 bytes apart. Entry 0 is the kernel's
//!   message-passing window (empty for now), entry 1 points to the level 1
//!   table that all CPUs share. It maps the system page, the callouts'
//!   devices and the bootstrap executables (the kernel itself).
//!
//! The kernel finds its page tables through *recursive* entries: L1[0]
//! points back at the L1 table and L2[0] at it too, so that the tables
//! appear as data in the kernel's address space. Because a table entry
//! reached that way is read as a page entry, table entries carry the same
//! attribute bits as page entries.

use crate::ram::Ram;

/// Page size: 4 KB.
pub const PAGE_SIZE: u64 = 4096;

// ANCHOR: address_space
/// Start of the kernel's half: L0 entry 1 of the TTBR1 range (40 bits).
pub const KERN_BASE: u64 = 0xffff_ff80_0000_0000;
/// Where startup's mappings start: L1 entry 1, L2 entry 1.
pub const STARTUP_BASE: u64 = KERN_BASE + (1 << 30) + (1 << 21);
/// Where mkifs links the bootstrap executables: L1 entry 1, L2 entry 256.
pub const BOOTPGM_BASE: u64 = KERN_BASE + (1 << 30) + (256 << 21);
/// End of the TTBR0 range (39 bits).
pub const USER_END: u64 = 1 << 39;
// ANCHOR_END: address_space

/// Distance between two CPUs' level 0 tables.
pub const L0_SPACING: u64 = 64;

/// What a mapping allows, and what it maps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prot {
    pub write: bool,
    pub exec: bool,
    /// Accessible from EL0 too.
    pub user: bool,
    /// Device registers rather than RAM.
    pub device: bool,
    /// Not cacheable (RAM), or strongly ordered (device).
    pub nocache: bool,
}

impl Prot {
    pub const RO: Prot = Prot {
        write: false,
        exec: false,
        user: false,
        device: false,
        nocache: false,
    };
    pub const RW: Prot = Prot {
        write: true,
        ..Prot::RO
    };
    pub const RX: Prot = Prot {
        exec: true,
        ..Prot::RO
    };
    pub const RWX: Prot = Prot {
        write: true,
        exec: true,
        ..Prot::RO
    };
    pub const DEVICE: Prot = Prot {
        write: true,
        device: true,
        nocache: true,
        ..Prot::RO
    };
}

// Page and table descriptor bits.
const PTE_VALID: u64 = 3;
const PTE_AF: u64 = 1 << 10;
const PTE_NG: u64 = 1 << 11;
const PTE_PXN: u64 = 1 << 53;
const PTE_UXN: u64 = 1 << 54;
const fn ap(x: u64) -> u64 {
    x << 6
}
const fn attr(index: u64) -> u64 {
    index << 2
}
const PTE_ISH: u64 = 3 << 8;
const PADDR_MASK: u64 = 0x0000_ffff_ffff_f000;

// ANCHOR: registers
/// `MAIR_EL1`: memory attribute indexes 0 device-nGnRnE, 1 device-nGnRE,
/// 2 normal write-through, 3 normal write-back no-allocate, 4 normal
/// non-cacheable, 7 normal write-back allocate (the ARMv7 TEX/C/B order).
pub const MAIR_EL1: u64 = 0xff00_0044_eeaa_0400;
/// `SCTLR_EL1` with the MMU on: caches on (C, I), EL0 may use cache
/// maintenance, WFE/WFI and DC ZVA (UCI, nTWE, nTWI, UCT, DZE), stack
/// alignment checks (SA, SA0), SETEND off (SED), plus the RES1 bits.
pub const SCTLR_EL1: u64 = 0x34d5_d91d;

/// `TCR_EL1`: 39-bit TTBR0 and 40-bit TTBR1 ranges, 4 KB granules, page
/// table walks cacheable write-back and inner shareable; 16-bit ASIDs if
/// the CPU has them; the physical address size the CPU has.
pub fn tcr_el1(mmfr0: u64) -> u64 {
    let t0sz = 64 - 39;
    let t1sz = (64 - 40) << 16;
    let tg1_4k = 2 << 30;
    let walk_wbwa_ish = (1 << 8) | (1 << 10) | (3 << 12) | (1 << 24) | (1 << 26) | (3 << 28);
    let asid16 = if (mmfr0 >> 4) & 0xf != 0 { 1 << 36 } else { 0 };
    let ips = (mmfr0 & 0x7) << 32;
    t0sz | t1sz | tg1_4k | walk_wbwa_ish | asid16 | ips
}
// ANCHOR_END: registers

/// The page tables being built, and the registers that use them.
pub struct Mmu {
    /// The TTBR0 level 1 table.
    pub ttbr0: u64,
    /// The shared kernel level 1 table.
    l1: u64,
    /// CPU 0's level 0 table, physical and in the kernel's address space.
    l0_paddr: u64,
    pub l0_vaddr: u64,
    num_cpu: usize,
    /// Map nothing global (`-E meltdown`): the kernel then unmaps itself
    /// from user processes' view, against Meltdown.
    pub non_global: bool,
    /// Next free address in the startup mapping range.
    free_vaddr: u64,
    pub tcr: u64,
}

/// Read or write a page table entry, with the MMU off.
fn entry(table: u64, index: u64) -> *mut u64 {
    (table + index * 8) as *mut u64
}

fn get(table: u64, index: u64) -> u64 {
    // SAFETY: tables are pages of RAM that startup allocated.
    unsafe { core::ptr::read_volatile(entry(table, index)) }
}

fn set(table: u64, index: u64, value: u64) {
    // SAFETY: as in get().
    unsafe { core::ptr::write_volatile(entry(table, index), value) }
}

impl Mmu {
    // ANCHOR: pte
    /// A page entry for `paddr` (also used for tables, see the module
    /// documentation).
    fn pte(&self, paddr: u64, prot: Prot) -> u64 {
        let mut pte = paddr | PTE_VALID | PTE_AF;
        pte |= match (prot.write, prot.user) {
            (true, false) => ap(0),
            (true, true) => ap(1),
            (false, false) => ap(2),
            (false, true) => ap(3),
        };
        if !prot.exec {
            pte |= PTE_UXN | PTE_PXN;
        }
        pte |= match (prot.device, prot.nocache) {
            (true, true) => attr(0),
            (true, false) => attr(1),
            (false, true) => attr(4),
            (false, false) => attr(7),
        };
        // Normal memory is shared between the CPUs.
        if !prot.device && self.num_cpu > 1 {
            pte |= PTE_ISH;
        }
        if self.non_global {
            pte |= PTE_NG;
        }
        pte
    }
    // ANCHOR_END: pte

    fn alloc_table(ram: &mut Ram) -> u64 {
        let pa = ram
            .alloc(PAGE_SIZE, PAGE_SIZE)
            .unwrap_or_else(|| crate::crash!("no RAM for page tables"));
        // SAFETY: a page of RAM startup just took; the MMU is off.
        unsafe { core::ptr::write_bytes(pa as *mut u8, 0, PAGE_SIZE as usize) };
        pa
    }

    fn new_table(&self, ram: &mut Ram) -> u64 {
        self.pte(Self::alloc_table(ram), Prot::RW)
    }

    /// Allocate the initial tables for `num_cpu` CPUs.
    pub fn new(ram: &mut Ram, num_cpu: usize, mmfr0: u64, non_global: bool) -> Self {
        let mut mmu = Mmu {
            ttbr0: Self::alloc_table(ram),
            l1: 0,
            l0_paddr: 0,
            l0_vaddr: 0,
            num_cpu,
            non_global,
            free_vaddr: STARTUP_BASE,
            tcr: tcr_el1(mmfr0),
        };
        // The shared L1, mapped at L1[0] (L1_BASE), and the L2 for
        // startup's mappings at L1[1], with L2[0] back at L1 (L2_BASE).
        let l1pte = mmu.new_table(ram);
        mmu.l1 = l1pte & PADDR_MASK;
        set(mmu.l1, 0, l1pte);
        let l2pte = mmu.new_table(ram);
        set(mmu.l1, 1, l2pte);
        set(l2pte & PADDR_MASK, 0, l1pte);
        // One L0 table per CPU, 64 bytes apart, each with entry 1 (the
        // kernel's 512 GB) at the shared L1.
        let l0_size = (num_cpu as u64 * L0_SPACING).next_multiple_of(PAGE_SIZE);
        mmu.l0_paddr = ram
            .alloc(l0_size, PAGE_SIZE)
            .unwrap_or_else(|| crate::crash!("no RAM for L0 tables"));
        // SAFETY: RAM startup just took; the MMU is off.
        unsafe { core::ptr::write_bytes(mmu.l0_paddr as *mut u8, 0, l0_size as usize) };
        for cpu in 0..num_cpu as u64 {
            set(mmu.l0_paddr + cpu * L0_SPACING, 1, l1pte);
        }
        mmu.l0_vaddr = mmu.map(ram, None, mmu.l0_paddr, l0_size, Prot::RW);
        mmu
    }

    /// `TTBR1_EL1` for `cpu`: its level 0 table.
    pub fn ttbr1(&self, cpu: usize) -> u64 {
        self.l0_paddr + cpu as u64 * L0_SPACING
    }

    // ANCHOR: mmu_map
    /// Map `size` bytes at `paddr` with `prot`, at `vaddr`, or at the next
    /// free address of the startup range if `vaddr` is `None`. Returns the
    /// virtual address (with `paddr`'s offset in its page).
    pub fn map(
        &mut self,
        ram: &mut Ram,
        vaddr: Option<u64>,
        paddr: u64,
        size: u64,
        prot: Prot,
    ) -> u64 {
        let off = paddr % PAGE_SIZE;
        let size = (size + off).next_multiple_of(PAGE_SIZE);
        let vaddr = match vaddr {
            Some(v) => {
                assert_eq!(
                    v % PAGE_SIZE,
                    off,
                    "map: {v:#x} and {paddr:#x} differ in page offset"
                );
                v - off
            }
            None => {
                let v = self.free_vaddr;
                self.free_vaddr += size;
                v
            }
        };
        let l1 = if vaddr < USER_END {
            assert!(
                vaddr + size <= USER_END,
                "map: {vaddr:#x} crosses the end of TTBR0"
            );
            self.ttbr0
        } else {
            assert!(
                vaddr >= STARTUP_BASE,
                "map: {vaddr:#x} below the startup range"
            );
            self.l1
        };
        let mut pte = self.pte(paddr - off, prot);
        for va in (vaddr..vaddr + size).step_by(PAGE_SIZE as usize) {
            let l2 = self.table(ram, l1, (va >> 30) & 0x1ff);
            let l3 = self.table(ram, l2, (va >> 21) & 0x1ff);
            set(l3, (va >> 12) & 0x1ff, pte);
            pte += PAGE_SIZE;
        }
        vaddr + off
    }
    // ANCHOR_END: mmu_map

    /// The table at `index` of `table`, allocated if missing.
    fn table(&self, ram: &mut Ram, table: u64, index: u64) -> u64 {
        let e = get(table, index);
        if e != 0 {
            return e & PADDR_MASK;
        }
        let t = self.new_table(ram);
        set(table, index, t);
        t & PADDR_MASK
    }

    /// The physical address `vaddr` is mapped to.
    pub fn vaddr_to_paddr(&self, vaddr: u64) -> Option<u64> {
        let l1 = if vaddr < USER_END {
            self.ttbr0
        } else {
            self.l1
        };
        let mut table = l1;
        for shift in [30, 21] {
            let e = get(table, (vaddr >> shift) & 0x1ff);
            if e & 3 != 3 {
                return None;
            }
            table = e & PADDR_MASK;
        }
        let e = get(table, (vaddr >> 12) & 0x1ff);
        (e & 3 == 3).then_some((e & PADDR_MASK) | (vaddr % PAGE_SIZE))
    }
}

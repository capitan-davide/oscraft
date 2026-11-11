//! The jump into the kernel: turn the MMU on and enter a bootstrap
//! executable (or, on a secondary CPU, the `smp_spin` callout).
//!
//! Until here every CPU runs with the MMU off. `vstart` loads the memory
//! attributes, the translation control and both table bases, turns the
//! MMU and caches on, and jumps. The instruction after the MMU comes on
//! is fetched through the TTBR0 1:1 map of startup; the jump goes to the
//! kernel's half of the address space.

use super::mmu::{MAIR_EL1, SCTLR_EL1};
use core::arch::global_asm;

/// The registers [`vstart`] loads.
#[repr(C)]
pub struct MmuRegs {
    pub mair: u64,
    pub tcr: u64,
    pub ttbr0: u64,
    pub ttbr1: u64,
    pub sctlr: u64,
}

impl MmuRegs {
    pub fn new(tcr: u64, ttbr0: u64, ttbr1: u64) -> Self {
        Self {
            mair: MAIR_EL1,
            tcr,
            ttbr0,
            ttbr1,
            sctlr: SCTLR_EL1,
        }
    }
}

// ANCHOR: vstart
// __oscraft_vstart(syspage, entry, cpu, &MmuRegs): never returns.
global_asm!(
    ".section .text.__oscraft_vstart, \"ax\"",
    ".global __oscraft_vstart",
    "__oscraft_vstart:",
    // Nothing cached from before: no stale instructions or translations.
    "    ic      iallu",
    "    tlbi    vmalle1",
    "    dsb     sy",
    "    ldp     x4, x5, [x3]", // MAIR, TCR
    "    msr     mair_el1, x4",
    "    msr     tcr_el1, x5",
    "    ldp     x4, x5, [x3, #16]", // TTBR0, TTBR1
    "    msr     ttbr0_el1, x4",
    "    msr     ttbr1_el1, x5",
    // FP/SIMD, SVE and trace trap until the kernel enables them.
    "    msr     cpacr_el1, xzr",
    "    ldr     x4, [x3, #32]", // SCTLR
    "    isb",
    "    msr     sctlr_el1, x4",
    "    isb",
    // entry(syspage, cpu)
    "    mov     x4, x1",
    "    mov     w1, w2",
    "    br      x4",
);
// ANCHOR_END: vstart

unsafe extern "C" {
    fn __oscraft_vstart(syspage: u64, entry: u64, cpu: u64, regs: &MmuRegs) -> !;
}

/// Turn the MMU on with `regs` and jump to `entry(syspage, cpu)`.
///
/// # Safety
///
/// The tables must map this code 1:1 and `entry` and `syspage` in the
/// kernel's half; nothing of startup is used after the jump.
pub unsafe fn vstart(syspage: u64, entry: u64, cpu: usize, regs: &MmuRegs) -> ! {
    // SAFETY: as the caller guarantees.
    unsafe { __oscraft_vstart(syspage, entry, cpu as u64, regs) }
}

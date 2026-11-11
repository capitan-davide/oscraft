//! AArch64 support.

use core::arch::{asm, global_asm};

pub mod context;
pub mod exception;
pub mod intrgen;
pub mod kercall;
pub mod mmu;
pub mod timer;
pub mod uspace;

global_asm!(include_str!("boot.s"));

/// Unmask IRQs on this CPU.
pub fn irq_enable() {
    // SAFETY: changes only the interrupt mask.
    unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) };
}

/// Unmask interrupts for a moment, from a masked state and back to it: an
/// interrupt pending meanwhile is taken here (`sched::preemption_point`).
pub fn irq_window() {
    // SAFETY: changes only the interrupt mask; the ISB makes the unmask
    // take effect before the mask is set again. Not `nomem`: memory the
    // interrupt changes is read again after.
    unsafe { asm!("msr daifclr, #2", "isb", "msr daifset, #2", options(nostack)) };
}

/// Wait for an interrupt.
pub fn wait_for_interrupt() {
    // SAFETY: wfi only waits.
    unsafe { asm!("wfi", options(nomem, nostack, preserves_flags)) };
}

/// Mask IRQs and return the previous `DAIF` state.
pub fn irq_save() -> u64 {
    let daif: u64;
    // SAFETY: changes only the interrupt mask.
    unsafe { asm!("mrs {}, daif", "msr daifset, #2", out(reg) daif, options(nomem, nostack)) };
    daif
}

/// Restore an interrupt mask returned by `irq_save`.
pub fn irq_restore(daif: u64) {
    // SAFETY: changes only the interrupt mask.
    unsafe { asm!("msr daif, {}", in(reg) daif, options(nomem, nostack)) };
}

/// Set this CPU's per-CPU data pointer (`TPIDR_EL1`).
pub fn set_percpu(ptr: u64) {
    // SAFETY: TPIDR_EL1 is reserved for the kernel's per-CPU pointer.
    unsafe { asm!("msr tpidr_el1, {}", in(reg) ptr, options(nomem, nostack)) };
}

/// This CPU's per-CPU data pointer.
pub fn percpu() -> u64 {
    let p: u64;
    // SAFETY: reading TPIDR_EL1 has no side effects.
    unsafe { asm!("mrs {}, tpidr_el1", out(reg) p, options(nomem, nostack, preserves_flags)) };
    p
}

/// Stop this core for good.
pub fn halt() -> ! {
    loop {
        // SAFETY: wfe only waits for an event.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

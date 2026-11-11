//! AArch64: the entry point, exception levels, caches and the MMU.

pub mod cache;
pub mod cpu;
pub mod entry;
pub mod exceptions;
pub mod gic_v2;
pub mod handoff;
pub mod intr;
pub mod mmu;
pub mod smp;

/// Prepare a CPU that runs at EL1 with a stack: report exceptions, and
/// drop stale cache lines (see [`cache`]). Called from `_start`.
#[unsafe(no_mangle)]
extern "C" fn __oscraft_cpu_init() {
    exceptions::install();
    cache::clean_invalidate_dcache();
    cache::invalidate_icache();
}

/// The exception level the CPU runs at.
pub fn current_el() -> u64 {
    let el: u64;
    // SAFETY: reading CurrentEL has no side effect.
    unsafe { core::arch::asm!("mrs {}, CurrentEL", out(reg) el) };
    el >> 2
}

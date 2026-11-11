//! ARM generic timer, EL1 virtual timer (`CNTV_*`). Architectural, so the
//! same on every AArch64 board; its interrupt number comes from the system
//! page (`qtime.intr`).

use core::arch::asm;

// ANCHOR: generic_timer
/// Counter frequency in Hz (`CNTFRQ_EL0`).
pub fn frequency() -> u64 {
    let f: u64;
    // SAFETY: reading CNTFRQ_EL0 has no side effects.
    unsafe { asm!("mrs {}, cntfrq_el0", out(reg) f, options(nomem, nostack)) };
    f
}

/// Current virtual count (`CNTVCT_EL0`).
pub fn counter() -> u64 {
    let c: u64;
    // SAFETY: reading the counter has no side effects; isb orders it.
    unsafe { asm!("isb", "mrs {}, cntvct_el0", out(reg) c, options(nomem, nostack)) };
    c
}

/// Raise the timer interrupt when the counter reaches `deadline`.
pub fn arm_at(deadline: u64) {
    // SAFETY: programs this CPU's virtual timer only.
    unsafe {
        asm!(
            "msr cntv_cval_el0, {}",
            "msr cntv_ctl_el0, {}",
            "isb",
            in(reg) deadline,
            in(reg) 1u64, // ENABLE, interrupt not masked
            options(nomem, nostack)
        );
    }
}
// ANCHOR_END: generic_timer

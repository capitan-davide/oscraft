//! Data cache maintenance by set/way.
//!
//! Startup runs with the MMU off, so its data accesses do not go through
//! the data cache. The cache may still hold lines from the boot loader,
//! though, and when the kernel turns the caches on, stale lines would hide
//! what startup wrote to memory. So each CPU cleans and invalidates its
//! data caches once, before it touches memory the kernel will use.

use core::arch::asm;

// ANCHOR: clean_invalidate_dcache
/// Clean and invalidate every data and unified cache level by set/way, up
/// to the level of coherence (`CLIDR_EL1.LoC`).
pub fn clean_invalidate_dcache() {
    let clidr: u64;
    // SAFETY: reading an ID register has no side effect.
    unsafe { asm!("mrs {}, clidr_el1", out(reg) clidr) };
    let loc = (clidr >> 24) & 0x7;
    for level in 0..loc {
        // Cache type of this level: 2 data, 3 separate I+D, 4 unified.
        let ctype = (clidr >> (level * 3)) & 0x7;
        if ctype < 2 {
            continue;
        }
        let ccsidr: u64;
        // SAFETY: selecting a cache level and reading its geometry only
        // changes CSSELR_EL1, which nothing else uses.
        unsafe {
            asm!(
                "msr csselr_el1, {sel}",
                "isb",
                "mrs {ccsidr}, ccsidr_el1",
                sel = in(reg) level << 1,
                ccsidr = out(reg) ccsidr,
            )
        };
        let line_shift = (ccsidr & 0x7) + 4;
        let ways = ((ccsidr >> 3) & 0x3ff) + 1;
        let sets = ((ccsidr >> 13) & 0x7fff) + 1;
        // The way number goes in the top bits of the set/way operand.
        let way_shift = (ways as u32 - 1).leading_zeros();
        for way in 0..ways {
            for set in 0..sets {
                let sw = (way << way_shift) | (set << line_shift) | (level << 1);
                // SAFETY: clean+invalidate writes dirty lines back and
                // drops them; with the MMU off, nothing relies on them.
                unsafe { asm!("dc cisw, {}", in(reg) sw) };
            }
        }
    }
    // SAFETY: barriers have no other effect.
    unsafe { asm!("dsb sy", "isb") };
}
// ANCHOR_END: clean_invalidate_dcache

/// Invalidate the instruction cache, for all CPUs.
pub fn invalidate_icache() {
    // SAFETY: invalidating the instruction cache only makes later fetches
    // read memory.
    unsafe { asm!("ic ialluis", "dsb ish", "isb") };
}

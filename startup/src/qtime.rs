//! The `qtime` section: the clocks.
//!
//! The kernel counts time with a free-running counter, `ClockCycles()`
//! (on AArch64, the generic timer's virtual count), and takes its system
//! tick from a per-CPU timer interrupt. qtime tells it the counter's
//! frequency, the tick's interrupt, and the counter's period as a decimal
//! fraction of a second.

use crate::abi::syspage::{QTIME_FLAG_GLOBAL_CLOCKCYCLES, QtimeEntry};

// ANCHOR: invert_timer_freq
/// The period of a `freq` Hz counter as `period * 10^scale` seconds, with
/// as many digits of `period` as fit (the `timer_period` and
/// `timer_scale` of qtime): 62.5 MHz is 16 * 10^-9 s.
pub fn invert_timer_freq(freq: u32) -> (u32, i32) {
    assert!(freq != 0, "timer frequency is zero");
    let (mut freq, mut scale) = (freq, 0);
    // So that rem * 10 cannot overflow.
    if freq > u32::MAX / 10 {
        scale -= 1;
        freq /= 10;
    }
    // Long division of 1 by freq, one decimal digit at a time.
    let (mut quot, mut period) = (1u32, 0u32);
    loop {
        period = period * 10 + quot / freq;
        if period >= u32::MAX / 10 - 10 {
            break;
        }
        let rem = quot % freq;
        if rem == 0 {
            break;
        }
        scale -= 1;
        quot = rem * 10;
    }
    (period, scale)
}
// ANCHOR_END: invert_timer_freq

/// The qtime section for a counter of `freq` Hz that is the same on all
/// CPUs, with the system tick on interrupt vector `intr`.
pub fn qtime(freq: u32, intr: i32) -> QtimeEntry {
    let (timer_period, timer_scale) = invert_timer_freq(freq);
    QtimeEntry {
        cycles_per_sec: u64::from(freq),
        timer_period,
        timer_scale,
        intr,
        epoch: 1970,
        flags: QTIME_FLAG_GLOBAL_CLOCKCYCLES,
        // The generic timer's compare value is 64 bits, signed.
        timer_load_max: i64::MAX as u64,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods() {
        // QEMU's raspi4b and many boards: 62.5 MHz, 16 ns.
        assert_eq!(invert_timer_freq(62_500_000), (16, -9));
        // The Raspberry Pi 4: 54 MHz, 18.518518... ns.
        assert_eq!(invert_timer_freq(54_000_000), (1_851_851_851, -17));
        assert_eq!(invert_timer_freq(1_000_000_000), (1, -9));
        assert_eq!(invert_timer_freq(1), (1, 0));
    }
}

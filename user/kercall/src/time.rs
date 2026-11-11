//! The time of day, for a server's timestamps.

use crate::neutrino::clock::ClockTime_r;

/// The realtime clock in nanoseconds (`ClockTime()`), or `u64::MAX` if
/// it cannot be read. `_which` is kept for oscraft-libc's signature.
pub fn _syspage_time(_which: u32) -> u64 {
    let mut now: u64 = 0;
    // SAFETY: the time is on this stack; no new time is set.
    if ClockTime_r(0, core::ptr::null(), &raw mut now) == 0 { now } else { u64::MAX }
}

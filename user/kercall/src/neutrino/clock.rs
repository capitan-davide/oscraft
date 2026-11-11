//! Clocks: the time, its adjustment, the tick's period, and per-thread and
//! per-process clocks.

use core::ffi::*;

use super::nr;
use crate::abi::*;

kercall! {
    /// `int ClockAdjust(clockid_t __id, const struct _clockadjust *_new, struct _clockadjust *__old)`
    errno ClockAdjust = nr::CLOCK_ADJUST;
    fn(id: ClockidT, new_: *const c_void, old: *mut c_void) -> c_int
}

kercall! {
    /// `int ClockAdjust_r(clockid_t __id, const struct _clockadjust *_new, struct _clockadjust *__old)`
    status ClockAdjust_r = nr::CLOCK_ADJUST;
    fn(id: ClockidT, new_: *const c_void, old: *mut c_void) -> c_int
}

kercall! {
    /// `int ClockId(pid_t __pid, int __tid)`
    errno ClockId = nr::CLOCK_ID;
    fn(pid: PidT, tid: c_int) -> c_int
}

kercall! {
    /// `int ClockId_r(pid_t __pid, int __tid)`
    neg ClockId_r = nr::CLOCK_ID;
    fn(pid: PidT, tid: c_int) -> c_int
}

kercall! {
    /// `int ClockPeriod(clockid_t __id, const struct _clockperiod *_new, struct _clockperiod *__old, int __reserved)`
    errno ClockPeriod = nr::CLOCK_PERIOD;
    fn(id: ClockidT, new_: *const c_void, old: *mut c_void, reserved: c_int) -> c_int
}

kercall! {
    /// `int ClockPeriod_r(clockid_t __id, const struct _clockperiod *_new, struct _clockperiod *__old, int __reserved)`
    status ClockPeriod_r = nr::CLOCK_PERIOD;
    fn(id: ClockidT, new_: *const c_void, old: *mut c_void, reserved: c_int) -> c_int
}

kercall! {
    /// `int ClockTime(clockid_t __id, const _Uint64t *__new, _Uint64t *__old)`
    errno ClockTime = nr::CLOCK_TIME;
    fn(id: ClockidT, new_: *const u64, old: *mut u64) -> c_int
}

kercall! {
    /// `int ClockTime_r(clockid_t __id, const _Uint64t *__new, _Uint64t *__old)`
    status ClockTime_r = nr::CLOCK_TIME;
    fn(id: ClockidT, new_: *const u64, old: *mut u64) -> c_int
}

/// `ClockCycles()`: the free-running cycle counter. `<sys/neutrino.h>`
/// defines it inline, per architecture, so QNX's libc has no symbol for
/// it; this is the platform's Rust API for the same.
pub use crate::arch::clock_cycles;

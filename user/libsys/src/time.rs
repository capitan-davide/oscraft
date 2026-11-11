//! The clocks, and the CPUs.

use core::ffi::c_int;

use proto::errno::{EINVAL, ERANGE};

use crate::errno::fail;
use crate::kercall::clock_time;

/// FreeBSD's clock IDs (`<sys/_clock_id.h>`) and QNX's (`<time.h>`).
const BSD_CLOCK_REALTIME: c_int = 0;
const BSD_CLOCK_VIRTUAL: c_int = 1;
const BSD_CLOCK_PROF: c_int = 2;
const BSD_CLOCK_MONOTONIC: c_int = 4;
const BSD_CLOCK_UPTIME: c_int = 5;
const BSD_CLOCK_UPTIME_PRECISE: c_int = 7;
const BSD_CLOCK_UPTIME_FAST: c_int = 8;
const BSD_CLOCK_REALTIME_PRECISE: c_int = 9;
const BSD_CLOCK_REALTIME_FAST: c_int = 10;
const BSD_CLOCK_MONOTONIC_PRECISE: c_int = 11;
const BSD_CLOCK_MONOTONIC_FAST: c_int = 12;
const BSD_CLOCK_SECOND: c_int = 13;
const BSD_CLOCK_THREAD_CPUTIME_ID: c_int = 14;
const BSD_CLOCK_PROCESS_CPUTIME_ID: c_int = 15;
const QNX_CLOCK_REALTIME: c_int = 0;
const QNX_CLOCK_MONOTONIC: c_int = 2;
const QNX_CLOCK_PROCESS_CPUTIME_ID: c_int = 3;
const QNX_CLOCK_THREAD_CPUTIME_ID: c_int = 4;

/// QNX's clock for FreeBSD's `id`.
pub fn qnx_clock(id: c_int) -> Option<c_int> {
    Some(match id {
        BSD_CLOCK_REALTIME | BSD_CLOCK_REALTIME_PRECISE | BSD_CLOCK_REALTIME_FAST | BSD_CLOCK_SECOND => {
            QNX_CLOCK_REALTIME
        }
        BSD_CLOCK_MONOTONIC
        | BSD_CLOCK_MONOTONIC_PRECISE
        | BSD_CLOCK_MONOTONIC_FAST
        | BSD_CLOCK_UPTIME
        | BSD_CLOCK_UPTIME_PRECISE
        | BSD_CLOCK_UPTIME_FAST => QNX_CLOCK_MONOTONIC,
        BSD_CLOCK_PROCESS_CPUTIME_ID | BSD_CLOCK_VIRTUAL | BSD_CLOCK_PROF => QNX_CLOCK_PROCESS_CPUTIME_ID,
        BSD_CLOCK_THREAD_CPUTIME_ID => QNX_CLOCK_THREAD_CPUTIME_ID,
        _ => return None,
    })
}

/// `clock_gettime(id, tp)`: `ClockTime()` of QNX's clock, in nanoseconds.
///
/// # Safety
///
/// `tp` points to a `struct timespec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_clock_gettime(id: c_int, tp: *mut [i64; 2]) -> c_int {
    let Some(clock) = qnx_clock(id) else {
        return fail(EINVAL) as c_int;
    };
    match clock_time(clock) {
        Ok(ns) => {
            // SAFETY: the caller's guarantee.
            unsafe { tp.write([(ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as i64]) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `clock_getres(id, tp)`: the clocks tick in nanoseconds as far as a
/// program can tell (`ClockTime()` interpolates between ticks).
///
/// # Safety
///
/// `tp` points to a `struct timespec`, or is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_clock_getres(id: c_int, tp: *mut [i64; 2]) -> c_int {
    if qnx_clock(id).is_none() {
        return fail(EINVAL) as c_int;
    }
    if !tp.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { tp.write([0, 1]) };
    }
    0
}

/// `cpuset_getaffinity(level, which, id, setsize, mask)`: every CPU, for
/// any level and any target; no affinity is kept per thread yet.
///
/// # Safety
///
/// `mask` is writable for `setsize` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_cpuset_getaffinity(level: c_int, which: c_int, id: i64, setsize: usize, mask: *mut u8) -> c_int {
    const CPU_LEVEL_WHICH: c_int = 3;
    const CPU_WHICH_TID: c_int = 1;
    const TCTL_RUNMASK_GET_AND_SET_64: u32 = 7;
    let ncpus = crate::sysctl::ncpus().unwrap_or(1) as usize;
    if setsize * 8 < ncpus {
        return fail(ERANGE) as c_int;
    }
    let mut m: u64 = if ncpus >= 64 { u64::MAX } else { (1u64 << ncpus) - 1 };
    if level == CPU_LEVEL_WHICH && which == CPU_WHICH_TID {
        // The thread's runmask (a new mask of 0: read only).
        let mut word = 0u64;
        // SAFETY: `word` is a word the kernel reads and writes.
        if unsafe { crate::kercall::thread_ctl_ext(0, if id == -1 { 0 } else { id as c_int }, TCTL_RUNMASK_GET_AND_SET_64, &mut word as *mut u64 as u64) }.is_ok() && word != 0 {
            m = word;
        }
    }
    for i in 0..setsize {
        // SAFETY: the caller's guarantee.
        unsafe { mask.add(i).write(if i < 8 { (m >> (8 * i)) as u8 } else { 0 }) };
    }
    0
}

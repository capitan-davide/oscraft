//! Threads, as libthr asks the kernel for them (`<sys/thr.h>`): over
//! OsCraft's own calls for the FreeBSD personality (`kercall`), where
//! QNX's calls assume QNX's thread-local block.

use core::ffi::{c_char, c_int, c_void};

use proto::errno::EINVAL;

use crate::errno::fail;
use crate::kercall::{futex_wake, thr_new, thr_self, thread_ctl_ext, thread_destroy_all};

/// `struct thr_param`'s size, the only one the kernel takes.
const THR_PARAM_SIZE: usize = 0x68;

/// `thr_new(param, size)`.
///
/// # Safety
///
/// `param` points to a `struct thr_param` of `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_thr_new(param: *const c_void, size: c_int) -> c_int {
    if size as usize != THR_PARAM_SIZE {
        return fail(EINVAL) as c_int;
    }
    // The thread's scheduling, when the caller sets it (`param.rtp`, at
    // 0x48 of FreeBSD's `struct thr_param`): the kernel creates the thread
    // with it, as FreeBSD's does. Set afterwards, the thread could run,
    // and wait for a mutex, at the caller's priority first.
    // SAFETY: `param` has that layout.
    let rtp = unsafe { param.cast::<u8>().add(0x48).cast::<*const [u16; 2]>().read() };
    let sched = if rtp.is_null() {
        None
    } else {
        // SAFETY: the caller's `struct rtprio`.
        let [class, prio] = unsafe { rtp.read() };
        match rtp_sched(class, prio) {
            Some(s) => Some(s),
            None => return fail(EINVAL) as c_int,
        }
    };
    // SAFETY: the caller's guarantee.
    match unsafe { thr_new(param, size as usize, sched) } {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// FreeBSD's `struct rtprio` as the kernel's policy and priority
/// (`SchedSet()`'s); None for a class there isn't.
fn rtp_sched(class: u16, prio: u16) -> Option<(u32, u32)> {
    match class {
        RTP_PRIO_FIFO | RTP_PRIO_REALTIME if u32::from(prio) <= RTP_PRIO_MAX => {
            Some((if class == RTP_PRIO_FIFO { QNX_SCHED_FIFO } else { QNX_SCHED_RR }, RT_BASE + RTP_PRIO_MAX - u32::from(prio)))
        }
        RTP_PRIO_NORMAL => Some((QNX_SCHED_OTHER, DEFAULT_PRIORITY)),
        RTP_PRIO_IDLE => Some((QNX_SCHED_OTHER, 1)),
        _ => None,
    }
}

/// `thr_self(id)`.
///
/// # Safety
///
/// `id` points to a long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_thr_self(id: *mut i64) -> c_int {
    match thr_self() {
        Ok(t) => {
            // SAFETY: the caller's guarantee.
            unsafe { id.write(t) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `thr_exit(state)`: `*state` set to 1 (libthr's TID_TERMINATED, what
/// FreeBSD's kernel stores) and its waiters woken (a join waits on it),
/// then the thread destroyed. The last thread's end is the process's.
///
/// # Safety
///
/// `state` points to a long, or is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_thr_exit(state: *mut i64) -> ! {
    if !state.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { state.write(1) };
        futex_wake(state.cast(), u32::MAX);
    }
    let tid = crate::kercall::kernel_tid(thr_self().unwrap_or(0));
    loop {
        crate::kercall::thread_destroy(tid, -1, 0);
        // Only the process's last thread fails to end: then end it.
        thread_destroy_all(0);
    }
}

/// `thr_kill(id, sig)`: `SignalKill()` to the thread, with QNX's number.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_thr_kill(id: i64, sig: c_int) -> c_int {
    match crate::kercall::signal_kill(0, crate::kercall::kernel_tid(id), sig, 0, 0) {
        Ok(()) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `sigfastblock(cmd, ptr)`: the kernel keeps no fast-block word, so a
/// thread's signals are only ever deferred by `sigprocmask()`; libthr's
/// word is accepted and never marked pending, which is what it does
/// without the feature.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sigfastblock(_cmd: c_int, _ptr: *mut c_void) -> c_int {
    0
}

/// FreeBSD's real-time priorities on QNX's: `rtprio` class REALTIME
/// (SCHED_RR) and FIFO run at QNX priority `RT_BASE + sched_priority`
/// (sched_priority = 31 - rtp.prio, 0..31), above the default
/// priority of a time-sharing (NORMAL) thread, 10; an IDLE thread runs
/// at 1. The reverse mapping (RTP_LOOKUP) reports the base priority, as
/// FreeBSD's `pri_to_rtp`.
const RT_BASE: u32 = 11;
const RTP_PRIO_MAX: u32 = 31;
const RTP_PRIO_REALTIME: u16 = 2;
const RTP_PRIO_NORMAL: u16 = 3;
const RTP_PRIO_IDLE: u16 = 4;
const RTP_PRIO_FIFO: u16 = 10;
const RTP_LOOKUP: c_int = 0;
const RTP_SET: c_int = 1;
const QNX_SCHED_FIFO: u32 = 1;
const QNX_SCHED_RR: u32 = 2;
const QNX_SCHED_OTHER: u32 = 3;
const DEFAULT_PRIORITY: u32 = 10;

/// `rtprio_thread(function, lwpid, rtp)`: a thread's scheduling class
/// and priority (`SchedGet()`/`SchedSet()`), `lwpid` 0 for the caller.
///
/// # Safety
///
/// `rtp` points at a `struct rtprio` (two `u_short`s).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_rtprio_thread(function: c_int, lwpid: c_int, rtp: *mut [u16; 2]) -> c_int {
    let mut param = [0u8; 48];
    match function {
        RTP_LOOKUP => {
            let policy = match crate::kercall::sched_get(0, crate::kercall::kernel_tid(i64::from(lwpid)), &mut param) {
                Ok(p) => p,
                Err(e) => return fail(e) as c_int,
            };
            let base = u32::from_le_bytes([param[0], param[1], param[2], param[3]]);
            let r = match policy {
                QNX_SCHED_FIFO | QNX_SCHED_RR if base >= RT_BASE => {
                    let class = if policy == QNX_SCHED_FIFO { RTP_PRIO_FIFO } else { RTP_PRIO_REALTIME };
                    [class, RTP_PRIO_MAX.saturating_sub(base - RT_BASE) as u16]
                }
                _ if base < DEFAULT_PRIORITY => [RTP_PRIO_IDLE, 0],
                _ => [RTP_PRIO_NORMAL, 0],
            };
            // SAFETY: the caller's guarantee.
            unsafe { rtp.write(r) };
            0
        }
        RTP_SET => {
            // SAFETY: the caller's guarantee.
            let [class, prio] = unsafe { rtp.read() };
            let Some((policy, priority)) = rtp_sched(class, prio) else {
                return fail(EINVAL) as c_int;
            };
            param[0..4].copy_from_slice(&priority.to_le_bytes());
            match crate::kercall::sched_set(0, crate::kercall::kernel_tid(i64::from(lwpid)), policy, &param) {
                Ok(()) => 0,
                Err(e) => fail(e) as c_int,
            }
        }
        _ => fail(EINVAL) as c_int,
    }
}

/// `sched_get_priority_max(policy)`: 31 for the real-time policies, as
/// FreeBSD's `ksched_get_priority_max`; SCHED_OTHER's range is its
/// time-sharing span.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sched_get_priority_max(policy: c_int) -> c_int {
    match policy {
        1 | 3 => RTP_PRIO_MAX as c_int, // SCHED_FIFO, SCHED_RR
        2 => 167,                        // SCHED_OTHER: PRI_MAX_TIMESHARE - PRI_MIN_TIMESHARE
        _ => fail(EINVAL) as c_int,
    }
}

/// `sched_get_priority_min(policy)`: 0 for every policy.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sched_get_priority_min(policy: c_int) -> c_int {
    match policy {
        1..=3 => 0,
        _ => fail(EINVAL) as c_int,
    }
}

const TCTL_RUNMASK_GET_AND_SET_64: u32 = 7;
const TCTL_NAME: u32 = 11;
/// FreeBSD's thread names are MAXCOMLEN (19) bytes; longer ones are cut.
const MAXCOMLEN: usize = 19;
const CPU_LEVEL_WHICH: c_int = 3;
const CPU_WHICH_TID: c_int = 1;

/// `thr_set_name(id, name)`: the thread's name, `_NTO_TCTL_NAME` through
/// `ThreadCtlExt()` (any thread of the process); a null name deletes it.
///
/// # Safety
///
/// `name` is a NUL-terminated string, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_thr_set_name(id: i64, name: *const c_char) -> c_int {
    // struct _thread_name { int new_name_len; int name_buf_len; char name_buf[]; }
    let mut buf = [0u8; 8 + MAXCOMLEN + 1];
    let mut len = 0usize;
    if !name.is_null() {
        while len < MAXCOMLEN {
            // SAFETY: the caller's guarantee: NUL-terminated.
            let b = unsafe { name.add(len).read() } as u8;
            if b == 0 {
                break;
            }
            buf[8 + len] = b;
            len += 1;
        }
    }
    buf[0..4].copy_from_slice(&(len as i32).to_le_bytes());
    buf[4..8].copy_from_slice(&((MAXCOMLEN + 1) as i32).to_le_bytes());
    // SAFETY: `buf` holds a `struct _thread_name` with room for the old name.
    match unsafe { thread_ctl_ext(0, crate::kercall::kernel_tid(id), TCTL_NAME, buf.as_mut_ptr() as u64) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// The thread a cpuset call names: `CPU_WHICH_TID` only, -1 the caller.
fn cpuset_thread(level: c_int, which: c_int, id: i64) -> Result<c_int, i32> {
    if level != CPU_LEVEL_WHICH || which != CPU_WHICH_TID {
        return Err(EINVAL);
    }
    Ok(if id == -1 { 0 } else { crate::kercall::kernel_tid(id) })
}

/// `cpuset_setaffinity(level, which, id, size, mask)`: the thread's
/// runmask (`_NTO_TCTL_RUNMASK_GET_AND_SET_64`); the kernel sets only the
/// caller's (ENOTSUP for another thread, as procnto).
///
/// # Safety
///
/// `mask` is readable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_cpuset_setaffinity(level: c_int, which: c_int, id: i64, size: usize, mask: *const u8) -> c_int {
    let tid = match cpuset_thread(level, which, id) {
        Ok(t) => t,
        Err(e) => return fail(e) as c_int,
    };
    let mut m: u64 = 0;
    for i in 0..size {
        // SAFETY: the caller's guarantee.
        let b = unsafe { mask.add(i).read() };
        if i < 8 {
            m |= u64::from(b) << (8 * i);
        } else if b != 0 {
            return fail(EINVAL) as c_int;
        }
    }
    if m == 0 {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: `m` is a word the kernel reads and writes.
    match unsafe { thread_ctl_ext(0, tid, TCTL_RUNMASK_GET_AND_SET_64, &mut m as *mut u64 as u64) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `thr_wake(id)`: ends a `thr_suspend()`, which nothing makes yet;
/// accepted.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_thr_wake(_id: i64) -> c_int {
    0
}

/// `getcontext(ucp)`: not filled yet (libthr's rtld hooks take one to
/// size it); accepted.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_getcontext(_ucp: *mut c_void) -> c_int {
    0
}

/// `syscall(number, ...)`: the few numbers libthr uses by number.
///
/// # Safety
///
/// Varargs as the number says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_syscall(number: c_int, _a1: u64, _a2: u64) -> c_int {
    const SYS_FORK: c_int = 2;
    const SYS_GETPID: c_int = 20;
    match number {
        // libthr's fork, which resolves no symbol before forking.
        SYS_FORK => crate::proc::__sys_fork(),
        SYS_GETPID => crate::process::__sys_getpid(),
        _ => {
            crate::errno::report("syscall by number", i64::from(number));
            fail(proto::errno::ENOSYS) as c_int
        }
    }
}

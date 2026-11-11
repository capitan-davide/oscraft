//! Timers and kernel timeouts.

use core::ffi::*;

use super::nr;
use crate::abi::*;

kercall! {
    /// `int TimerAlarm(clockid_t __id, const struct _itimer *__itime, struct _itimer *__otime)`
    errno TimerAlarm = nr::TIMER_ALARM;
    fn(id: ClockidT, itime: *const c_void, otime: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerAlarm_r(clockid_t __id, const struct _itimer *__itime, struct _itimer *__otime)`
    status TimerAlarm_r = nr::TIMER_ALARM;
    fn(id: ClockidT, itime: *const c_void, otime: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerCreate(clockid_t __id, const struct sigevent *__notify)`
    errno TimerCreate = nr::TIMER_CREATE;
    fn(id: ClockidT, notify: *const c_void) -> c_int
}

kercall! {
    /// `int TimerCreate_r(clockid_t __id, const struct sigevent *__notify)`
    neg TimerCreate_r = nr::TIMER_CREATE;
    fn(id: ClockidT, notify: *const c_void) -> c_int
}

kercall! {
    /// `int TimerDelegate(_Uint32t __action, _Int32t owner_cpu, _Int32t delegate_cpu)`
    errno TimerDelegate = nr::TIMER_DELEGATE;
    fn(action: u32, owner_cpu: i32, delegate_cpu: i32) -> c_int
}

kercall! {
    /// `int TimerDelegate_r(_Uint32t __action, _Int32t owner_cpu, _Int32t delegate_cpu)`
    neg TimerDelegate_r = nr::TIMER_DELEGATE;
    fn(action: u32, owner_cpu: i32, delegate_cpu: i32) -> c_int
}

kercall! {
    /// `int TimerDestroy(timer_t __id)`
    errno TimerDestroy = nr::TIMER_DESTROY;
    fn(id: TimerT) -> c_int
}

kercall! {
    /// `int TimerDestroy_r(timer_t __id)`
    status TimerDestroy_r = nr::TIMER_DESTROY;
    fn(id: TimerT) -> c_int
}

kercall! {
    /// `int TimerInfo(pid_t __pid, timer_t __id, int __flags, struct _timer_info *__info)`
    errno TimerInfo = nr::TIMER_INFO;
    fn(pid: PidT, id: TimerT, flags: c_int, info: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerInfo_r(pid_t __pid, timer_t __id, int __flags, struct _timer_info *__info)`
    neg TimerInfo_r = nr::TIMER_INFO;
    fn(pid: PidT, id: TimerT, flags: c_int, info: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerSettime(timer_t __id, int __flags, const struct _itimer *__itime, struct _itimer *__oitime)`
    errno TimerSettime = nr::TIMER_SETTIME;
    fn(id: TimerT, flags: c_int, itime: *const c_void, oitime: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerSettime_r(timer_t __id, int __flags, const struct _itimer *__itime, struct _itimer *__oitime)`
    status TimerSettime_r = nr::TIMER_SETTIME;
    fn(id: TimerT, flags: c_int, itime: *const c_void, oitime: *mut c_void) -> c_int
}

kercall! {
    /// `int TimerTimeout(clockid_t __id, _Uint32t __flags, const struct sigevent *__notify, const _Uint64t *__ntime, _Uint64t *__otime)`
    errno TimerTimeout = nr::TIMER_TIMEOUT;
    fn(id: ClockidT, flags: u32, notify: *const c_void, ntime: *const u64, otime: *mut u64) -> c_int
}

kercall! {
    /// `int TimerTimeout_r(clockid_t __id, _Uint32t __flags, const struct sigevent *__notify, const _Uint64t *__ntime, _Uint64t *__otime)`
    neg TimerTimeout_r = nr::TIMER_TIMEOUT;
    fn(id: ClockidT, flags: u32, notify: *const c_void, ntime: *const u64, otime: *mut u64) -> c_int
}

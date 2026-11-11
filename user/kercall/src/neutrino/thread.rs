//! Threads.

use core::ffi::*;

use super::nr;
use crate::abi::*;

kercall! {
    /// `int ThreadCancel(int __tid, void (*__canstub)(void))`
    errno ThreadCancel = nr::THREAD_CANCEL;
    fn(tid: c_int, canstub: *const c_void) -> c_int
}

kercall! {
    /// `int ThreadCancel_r(int __tid, void (*__canstub)(void))`
    status ThreadCancel_r = nr::THREAD_CANCEL;
    fn(tid: c_int, canstub: *const c_void) -> c_int
}

kercall! {
    /// `int ThreadCreate(pid_t __pid, void *(*__func)(void *__arg), void *__arg, const struct _thread_attr *__attr)`
    errno ThreadCreate = nr::THREAD_CREATE;
    fn(pid: PidT, func: *const c_void, arg: *mut c_void, attr: *const c_void) -> c_int
}

kercall! {
    /// `int ThreadCreate_r(pid_t __pid, void *(*__func)(void *__arg), void *__arg, const struct _thread_attr *__attr)`
    neg ThreadCreate_r = nr::THREAD_CREATE;
    fn(pid: PidT, func: *const c_void, arg: *mut c_void, attr: *const c_void) -> c_int
}

kercall! {
    /// `int ThreadCtl(int __cmd, void *__data)`
    errno ThreadCtl = nr::THREAD_CTL;
    fn(cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadCtl_r(int __cmd, void *__data)`
    status ThreadCtl_r = nr::THREAD_CTL;
    fn(cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadCtlExt(pid_t __pid, int __tid, int __cmd, void *__data)`
    errno ThreadCtlExt = nr::THREAD_CTLEXT;
    fn(pid: PidT, tid: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadCtlExt_r(pid_t __pid, int __tid, int __cmd, void *__data)`
    status ThreadCtlExt_r = nr::THREAD_CTLEXT;
    fn(pid: PidT, tid: c_int, cmd: c_int, data: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadDestroy(int __tid, int __priority, void *__status)`
    errno ThreadDestroy = nr::THREAD_DESTROY;
    fn(tid: c_int, priority: c_int, status: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadDestroy_r(int __tid, int __priority, void *__status)`
    status ThreadDestroy_r = nr::THREAD_DESTROY;
    fn(tid: c_int, priority: c_int, status: *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadDetach(int __tid)`
    errno ThreadDetach = nr::THREAD_DETACH;
    fn(tid: c_int) -> c_int
}

kercall! {
    /// `int ThreadDetach_r(int __tid)`
    status ThreadDetach_r = nr::THREAD_DETACH;
    fn(tid: c_int) -> c_int
}

kercall! {
    /// `int ThreadJoin(int __tid, void **__status)`
    errno ThreadJoin = nr::THREAD_JOIN;
    fn(tid: c_int, status: *mut *mut c_void) -> c_int
}

kercall! {
    /// `int ThreadJoin_r(int __tid, void **__status)`
    status ThreadJoin_r = nr::THREAD_JOIN;
    fn(tid: c_int, status: *mut *mut c_void) -> c_int
}

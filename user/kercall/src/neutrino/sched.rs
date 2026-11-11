//! Scheduling.

use core::ffi::*;

use super::nr;
use crate::abi::*;

kercall! {
    /// `int SchedCtl(int __cmd, void *__data, size_t __length)`
    errno SchedCtl = nr::SCHED_CTL;
    fn(cmd: c_int, data: *mut c_void, length: usize) -> c_int
}

kercall! {
    /// `int SchedCtl_r(int __cmd, void *__data, size_t __length)`
    status SchedCtl_r = nr::SCHED_CTL;
    fn(cmd: c_int, data: *mut c_void, length: usize) -> c_int
}

kercall! {
    /// `int SchedGet(pid_t __pid, int __tid, struct sched_param *__param)`
    errno SchedGet = nr::SCHED_GET;
    fn(pid: PidT, tid: c_int, param: *mut c_void) -> c_int
}

kercall! {
    /// `int SchedGet_r(pid_t __pid, int __tid, struct sched_param *__param)`
    neg SchedGet_r = nr::SCHED_GET;
    fn(pid: PidT, tid: c_int, param: *mut c_void) -> c_int
}

kercall! {
    /// `int SchedInfo(pid_t __pid, int __algorithm, struct _sched_info *__info)`
    errno SchedInfo = nr::SCHED_INFO;
    fn(pid: PidT, algorithm: c_int, info: *mut c_void) -> c_int
}

kercall! {
    /// `int SchedInfo_r(pid_t __pid, int __algorithm, struct _sched_info *__info)`
    status SchedInfo_r = nr::SCHED_INFO;
    fn(pid: PidT, algorithm: c_int, info: *mut c_void) -> c_int
}

kercall! {
    /// `int SchedSet(pid_t __pid, int __tid, int __algorithm, const struct sched_param *__param)`
    errno SchedSet = nr::SCHED_SET;
    fn(pid: PidT, tid: c_int, algorithm: c_int, param: *const c_void) -> c_int
}

kercall! {
    /// `int SchedSet_r(pid_t __pid, int __tid, int __algorithm, const struct sched_param *__param)`
    status SchedSet_r = nr::SCHED_SET;
    fn(pid: PidT, tid: c_int, algorithm: c_int, param: *const c_void) -> c_int
}

kercall! {
    /// `int SchedYield(void)`
    errno SchedYield = nr::SCHED_YIELD;
    fn() -> c_int
}

kercall! {
    /// `int SchedYield_r(void)`
    status SchedYield_r = nr::SCHED_YIELD;
    fn() -> c_int
}

// Functions built on other calls.

/// `_NTO_SYNC_JOB` (`<sys/target_nto.h>`): a job is a sync object of its own type.
const SYNC_JOB: u32 = 0xffff_fff9;
/// `CPUPAGE_CPU` (`<sys/syspage.h>`): the CPU page's field for the CPU's number.
const CPUPAGE_CPU: c_int = 3;

/// `int SchedJobCreate(nto_job_t *job)`.
#[unsafe(no_mangle)]
pub extern "C" fn SchedJobCreate(job: *mut c_void) -> c_int {
    super::sync::SyncTypeCreate(SYNC_JOB, job, core::ptr::null())
}

/// `SchedJobCreate()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn SchedJobCreate_r(job: *mut c_void) -> c_int {
    super::sync::SyncTypeCreate_r(SYNC_JOB, job, core::ptr::null())
}

/// `int SchedJobDestroy(nto_job_t *job)`.
#[unsafe(no_mangle)]
pub extern "C" fn SchedJobDestroy(job: *mut c_void) -> c_int {
    super::sync::SyncTypeDestroy(SYNC_JOB, job)
}

/// `SchedJobDestroy()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn SchedJobDestroy_r(job: *mut c_void) -> c_int {
    super::sync::SyncTypeDestroy_r(SYNC_JOB, job)
}

/// `unsigned SchedGetCpuNum(void)`: the number of the CPU the thread runs
/// on now, from the CPU page.
#[unsafe(no_mangle)]
pub extern "C" fn SchedGetCpuNum() -> c_uint {
    super::misc::__SysCpupageGet(CPUPAGE_CPU) as c_uint
}

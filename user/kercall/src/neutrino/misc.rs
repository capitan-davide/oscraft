//! Kernel pipes, the kernel's random-number seed, the caches, and trace
//! events.

use core::ffi::*;

use super::nr;

kercall! {
    /// `int PipeClose(int __pipeid)`
    errno PipeClose = nr::PIPE_CLOSE;
    fn(pipeid: c_int) -> c_int
}

kercall! {
    /// `int PipeClose_r(int __pipeid)`
    status PipeClose_r = nr::PIPE_CLOSE;
    fn(pipeid: c_int) -> c_int
}

kercall! {
    /// `int PipeCtl(int __scoid, union nto_pipe_ctl * __ctl)`
    errno PipeCtl = nr::PIPE_CTL;
    fn(scoid: c_int, ctl: *mut c_void) -> c_int
}

kercall! {
    /// `int PipeCtl_r(int __scoid, union nto_pipe_ctl * __ctl)`
    neg PipeCtl_r = nr::PIPE_CTL;
    fn(scoid: c_int, ctl: *mut c_void) -> c_int
}

kercall! {
    /// `int PipeOpen(int __scoid, int __pipeid, struct nto_pipe_cfg * __cfg, unsigned __flags)`
    errno PipeOpen = nr::PIPE_OPEN;
    fn(scoid: c_int, pipeid: c_int, cfg: *mut c_void, flags: c_uint) -> c_int
}

kercall! {
    /// `int PipeOpen_r(int __scoid, int __pipeid, struct nto_pipe_cfg * __cfg, unsigned __flags)`
    status PipeOpen_r = nr::PIPE_OPEN;
    fn(scoid: c_int, pipeid: c_int, cfg: *mut c_void, flags: c_uint) -> c_int
}

kercall! {
    /// `int SysSrandom(const _Uint64t *__seedp, _Uint64t *__entropy, _Sizet __elements)`
    errno SysSrandom = nr::SYS_SRANDOM;
    fn(seedp: *const u64, entropy: *mut u64, elements: usize) -> c_int
}

kercall! {
    /// `int SysSrandom_r(const _Uint64t *__seedp, _Uint64t *__entropy, _Sizet __elements)`
    status SysSrandom_r = nr::SYS_SRANDOM;
    fn(seedp: *const u64, entropy: *mut u64, elements: usize) -> c_int
}

kercall! {
    /// `intptr_t __SysCpupageGet(int index)`: a field of the calling CPU's
    /// page (`enum cpupage_index`, `<sys/syspage.h>`). Not in a public
    /// header, but QNX's library exports it and programs use it. The value
    /// is pointer-sized: the system page's address is one.
    errno __SysCpupageGet = nr::SYS_CPUPAGE_GET;
    fn(index: c_int) -> isize
}

kercall! {
    /// `int __SysCpupageSet(int index, intptr_t value)`: sets a field of the
    /// calling CPU's page. The value is pointer-sized: the process local
    /// storage's address is one.
    errno __SysCpupageSet = nr::SYS_CPUPAGE_SET;
    fn(index: c_int, value: isize) -> c_int
}

kercall! {
    /// `int CacheControl(void *__addr, size_t __len, unsigned __flags)`:
    /// cleans or invalidates the caches for a range.
    errno CacheControl = nr::CACHE_CONTROL;
    fn(addr: *mut c_void, len: usize, flags: c_uint) -> c_int
}

kercall! {
    /// `int CacheControl_r(void *__addr, size_t __len, unsigned __flags)`
    status CacheControl_r = nr::CACHE_CONTROL;
    fn(addr: *mut c_void, len: usize, flags: c_uint) -> c_int
}

kercall! {
    /// `int __traceevent(intptr_t *__data)`: an instrumented kernel's
    /// trace request, the code and its arguments.
    errno __traceevent = nr::TRACE_EVENT;
    fn(data: *const isize) -> c_int
}

kercall! {
    /// `int __traceevent_r(intptr_t *__data)`
    status __traceevent_r = nr::TRACE_EVENT;
    fn(data: *const isize) -> c_int
}

kercall! {
    /// `int __Ring0(void (*func)(void *), void *arg)`: runs `func(arg)` in
    /// the kernel's privilege, if the process may.
    errno __Ring0 = nr::RING0;
    fn(func: *const c_void, arg: *mut c_void) -> c_int
}

kercall! {
    /// `int __Ring0_r(void (*func)(void *), void *arg)`
    status __Ring0_r = nr::RING0;
    fn(func: *const c_void, arg: *mut c_void) -> c_int
}

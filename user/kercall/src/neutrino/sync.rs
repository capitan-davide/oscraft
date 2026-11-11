//! Synchronisation objects as the kernel implements them: mutexes,
//! condition variables, semaphores, read-write locks, barriers.

use core::ffi::*;

use super::nr;

kercall! {
    /// `int SyncBarrierWait(sync_t * __sync)`
    errno SyncBarrierWait = nr::SYNC_BARRIER_WAIT;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncBarrierWait_r(sync_t * __sync)`
    status SyncBarrierWait_r = nr::SYNC_BARRIER_WAIT;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncCondvarSignal(sync_t *__sync, int __all)`
    errno SyncCondvarSignal = nr::SYNC_CONDVAR_SIGNAL;
    fn(sync: *mut c_void, all: c_int) -> c_int
}

kercall! {
    /// `int SyncCondvarSignal_r(sync_t *__sync, int __all)`
    status SyncCondvarSignal_r = nr::SYNC_CONDVAR_SIGNAL;
    fn(sync: *mut c_void, all: c_int) -> c_int
}

kercall! {
    /// `int SyncCondvarWait(sync_t *__sync, sync_t *__mutex)`
    errno SyncCondvarWait = nr::SYNC_CONDVAR_WAIT;
    fn(sync: *mut c_void, mutex: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncCondvarWait_r(sync_t *__sync, sync_t *__mutex)`
    status SyncCondvarWait_r = nr::SYNC_CONDVAR_WAIT;
    fn(sync: *mut c_void, mutex: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncCtl(int __cmd, sync_t *__sync, void *__data)`
    errno SyncCtl = nr::SYNC_CTL;
    fn(cmd: c_int, sync: *mut c_void, data: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncCtl_r(int __cmd, sync_t *__sync, void *__data)`
    neg SyncCtl_r = nr::SYNC_CTL;
    fn(cmd: c_int, sync: *mut c_void, data: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncDestroy(sync_t *__sync)`
    errno SyncDestroy = nr::SYNC_DESTROY;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncDestroy_r(sync_t *__sync)`
    status SyncDestroy_r = nr::SYNC_DESTROY;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncMutexLock(sync_t *__sync)`
    errno SyncMutexLock = nr::SYNC_MUTEX_LOCK;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncMutexLock_r(sync_t *__sync)`
    status SyncMutexLock_r = nr::SYNC_MUTEX_LOCK;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncMutexUnlock(sync_t *__sync)`
    errno SyncMutexUnlock = nr::SYNC_MUTEX_UNLOCK;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncMutexUnlock_r(sync_t *__sync)`
    status SyncMutexUnlock_r = nr::SYNC_MUTEX_UNLOCK;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncRWLock(sync_t *__sync, unsigned __op)`
    errno SyncRWLock = nr::SYNC_RWLOCK;
    fn(sync: *mut c_void, op: c_uint) -> c_int
}

kercall! {
    /// `int SyncRWLock_r(sync_t *__sync, unsigned __op)`
    status SyncRWLock_r = nr::SYNC_RWLOCK;
    fn(sync: *mut c_void, op: c_uint) -> c_int
}

kercall! {
    /// `int SyncSemPost(sync_t *__sync)`
    errno SyncSemPost = nr::SYNC_SEM_POST;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncSemPost_r(sync_t *__sync)`
    status SyncSemPost_r = nr::SYNC_SEM_POST;
    fn(sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncSemWait(sync_t *__sync, int __tryto)`
    errno SyncSemWait = nr::SYNC_SEM_WAIT;
    fn(sync: *mut c_void, tryto: c_int) -> c_int
}

kercall! {
    /// `int SyncSemWait_r(sync_t *__sync, int __tryto)`
    status SyncSemWait_r = nr::SYNC_SEM_WAIT;
    fn(sync: *mut c_void, tryto: c_int) -> c_int
}

kercall! {
    /// `int SyncTypeCreate(unsigned __type, sync_t *__sync, const struct _sync_attr *__attr)`
    errno SyncTypeCreate = nr::SYNC_CREATE;
    fn(r#type: c_uint, sync: *mut c_void, attr: *const c_void) -> c_int
}

kercall! {
    /// `int SyncTypeCreate_r(unsigned __type, sync_t *__sync, const struct _sync_attr *__attr)`
    status SyncTypeCreate_r = nr::SYNC_CREATE;
    fn(r#type: c_uint, sync: *mut c_void, attr: *const c_void) -> c_int
}

kercall! {
    /// `int SyncTypeDestroy(uint32_t type, sync_t *__sync)`
    errno SyncTypeDestroy = nr::SYNC_TYPE_DESTROY;
    fn(r#type: u32, sync: *mut c_void) -> c_int
}

kercall! {
    /// `int SyncTypeDestroy_r(uint32_t type, sync_t *__sync)`
    status SyncTypeDestroy_r = nr::SYNC_TYPE_DESTROY;
    fn(r#type: u32, sync: *mut c_void) -> c_int
}

// Functions built on the calls above, with an argument fixed.

/// `_NTO_SYNC_MUTEX_FREE` (`<sys/target_nto.h>`): the type of a new mutex.
const SYNC_MUTEX_FREE: c_uint = 0;
/// `_NTO_SCTL_SETEVENT`: `SyncCtl()`'s command to set a mutex's event.
const SCTL_SETEVENT: c_int = 3;

/// `int SyncCreate(sync_t *sync, const struct _sync_attr *attr)`: creates a
/// mutex.
#[unsafe(no_mangle)]
pub extern "C" fn SyncCreate(sync: *mut c_void, attr: *const c_void) -> c_int {
    SyncTypeCreate(SYNC_MUTEX_FREE, sync, attr)
}

/// `SyncCreate()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn SyncCreate_r(sync: *mut c_void, attr: *const c_void) -> c_int {
    SyncTypeCreate_r(SYNC_MUTEX_FREE, sync, attr)
}

/// `int SyncMutexEvent(sync_t *sync, const struct sigevent *event)`: the
/// event the kernel delivers when the mutex's owner dies.
#[unsafe(no_mangle)]
pub extern "C" fn SyncMutexEvent(sync: *mut c_void, event: *const c_void) -> c_int {
    SyncCtl(SCTL_SETEVENT, sync, event.cast_mut())
}

/// `SyncMutexEvent()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn SyncMutexEvent_r(sync: *mut c_void, event: *const c_void) -> c_int {
    SyncCtl_r(SCTL_SETEVENT, sync, event.cast_mut())
}

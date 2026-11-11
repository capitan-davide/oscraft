//! Interrupts: attaching handlers and events, masking, waiting.

use core::ffi::*;

use super::nr;

kercall! {
    /// `int InterruptAttach(int __intr, const struct sigevent *(*__handler)(void *__area, int __id), const void *__area, int __size, unsigned __flags)`
    errno InterruptAttach = nr::INTERRUPT_ATTACH;
    fn(intr: c_int, handler: *const c_void, area: *const c_void, size: c_int, flags: c_uint) -> c_int
}

kercall! {
    /// `int InterruptAttach_r(int __intr, const struct sigevent *(*__handler)(void *__area, int __id), const void *__area, int __size, unsigned __flags)`
    neg InterruptAttach_r = nr::INTERRUPT_ATTACH;
    fn(intr: c_int, handler: *const c_void, area: *const c_void, size: c_int, flags: c_uint) -> c_int
}

kercall! {
    /// `int InterruptCtl(int __id, int __cmd, void *__data, size_t __len)`
    errno InterruptCtl = nr::INTERRUPT_CTL;
    fn(id: c_int, cmd: c_int, data: *mut c_void, len: usize) -> c_int
}

kercall! {
    /// `int InterruptCtl_r(int __id, int __cmd, void *__data, size_t __len)`
    status InterruptCtl_r = nr::INTERRUPT_CTL;
    fn(id: c_int, cmd: c_int, data: *mut c_void, len: usize) -> c_int
}

kercall! {
    /// `int InterruptDetach(int __id)`
    errno InterruptDetach = nr::INTERRUPT_DETACH;
    fn(id: c_int) -> c_int
}

kercall! {
    /// `int InterruptDetach_r(int __id)`
    status InterruptDetach_r = nr::INTERRUPT_DETACH;
    fn(id: c_int) -> c_int
}

kercall! {
    /// `int InterruptMask(int __intr, int __id)`
    errno InterruptMask = nr::INTERRUPT_MASK;
    fn(intr: c_int, id: c_int) -> c_int
}

kercall! {
    /// `int InterruptMask_r(int __intr, int __id)`
    neg InterruptMask_r = nr::INTERRUPT_MASK;
    fn(intr: c_int, id: c_int) -> c_int
}

kercall! {
    /// `int InterruptQuery(unsigned type, int id, unsigned long extra, struct interrupt_query_data *data)`
    errno InterruptQuery = nr::INTERRUPT_QUERY;
    fn(r#type: c_uint, id: c_int, extra: c_ulong, data: *mut c_void) -> c_int
}

kercall! {
    /// `int InterruptQuery_r(unsigned type, int id, unsigned long extra, struct interrupt_query_data *data)`
    neg InterruptQuery_r = nr::INTERRUPT_QUERY;
    fn(r#type: c_uint, id: c_int, extra: c_ulong, data: *mut c_void) -> c_int
}

kercall! {
    /// `int __InterruptWait(int __flags, const _Uint64t *__timeout)`
    errno __InterruptWait = nr::INTERRUPT_WAIT;
    fn(flags: c_int, timeout: *const u64) -> c_int
}

kercall! {
    /// `int __InterruptWait_r(int __flags, const _Uint64t *__timeout)`
    status __InterruptWait_r = nr::INTERRUPT_WAIT;
    fn(flags: c_int, timeout: *const u64) -> c_int
}

kercall! {
    /// `int __MuonCallInterruptWait(int __flags, const _Uint64t *__timeout)`:
    /// the fast wait.
    errno __MuonCallInterruptWait = nr::MUON_INTERRUPT_WAIT;
    fn(flags: c_int, timeout: *const u64) -> c_int
}

kercall! {
    /// `int __MuonCallInterruptWait_r(int __flags, const _Uint64t *__timeout)`
    status __MuonCallInterruptWait_r = nr::MUON_INTERRUPT_WAIT;
    fn(flags: c_int, timeout: *const u64) -> c_int
}

kercall! {
    /// `int InterruptUnblock(int __id)`
    errno InterruptUnblock = nr::INTERRUPT_UNBLOCK;
    fn(id: c_int) -> c_int
}

kercall! {
    /// `int InterruptUnblock_r(int __id)`
    status InterruptUnblock_r = nr::INTERRUPT_UNBLOCK;
    fn(id: c_int) -> c_int
}

kercall! {
    /// `int InterruptUnmask(int __intr, int __id)`
    errno InterruptUnmask = nr::INTERRUPT_UNMASK;
    fn(intr: c_int, id: c_int) -> c_int
}

kercall! {
    /// `int InterruptUnmask_r(int __intr, int __id)`
    neg InterruptUnmask_r = nr::INTERRUPT_UNMASK;
    fn(intr: c_int, id: c_int) -> c_int
}

// Functions built on InterruptAttach(): each is an attach with some of its
// arguments fixed. They are QNX's library's, with the same arguments.

/// `_NTO_INTR_FLAGS_PROCESS`: the handler is the process's, not the thread's.
const FLAGS_PROCESS: c_uint = 0x04;
/// `_NTO_INTR_FLAGS_ARRAY`: the handler returns an array of events.
const FLAGS_ARRAY: c_uint = 0x10;
/// `_NTO_INTR_FLAGS_EVENT_PRIORITY`: the size argument is the event's priority.
const FLAGS_EVENT_PRIORITY: c_uint = 0x100;
/// `_NTO_HOOK_TRACE`, `_NTO_HOOK_IDLE` with `_NTO_HOOK_IDLE2_FLAG`,
/// `_NTO_HOOK_OVERDRIVE`: synthetic interrupts (`_NTO_INTR_CLASS_SYNTHETIC`).
const HOOK_TRACE: c_int = 0x7fff_0000;
const HOOK_IDLE2: c_int = 0x7fff_8001;
const HOOK_OVERDRIVE: c_int = 0x7fff_0002;
/// `sizeof(struct sigevent)` (`<sys/siginfo.h>`).
const SIGEVENT_SIZE: c_int = 32;

/// `int InterruptAttachEvent(int intr, const struct sigevent *event, unsigned flags)`:
/// an attach with no handler: the kernel delivers `event` when the
/// interrupt fires.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachEvent(intr: c_int, event: *const c_void, flags: c_uint) -> c_int {
    InterruptAttach(intr, core::ptr::null(), event, SIGEVENT_SIZE, flags)
}

/// `InterruptAttachEvent()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachEvent_r(
    intr: c_int,
    event: *const c_void,
    flags: c_uint,
) -> c_int {
    InterruptAttach_r(intr, core::ptr::null(), event, SIGEVENT_SIZE, flags)
}

/// `int InterruptAttachEventPriority(int intr, const struct sigevent *event, unsigned prio, unsigned flags)`:
/// as `InterruptAttachEvent()`, with the event's priority in the size argument.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachEventPriority(
    intr: c_int,
    event: *const c_void,
    prio: c_uint,
    flags: c_uint,
) -> c_int {
    InterruptAttach(
        intr,
        core::ptr::null(),
        event,
        prio as c_int,
        flags | FLAGS_EVENT_PRIORITY,
    )
}

/// `InterruptAttachEventPriority()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachEventPriority_r(
    intr: c_int,
    event: *const c_void,
    prio: c_uint,
    flags: c_uint,
) -> c_int {
    InterruptAttach_r(
        intr,
        core::ptr::null(),
        event,
        prio as c_int,
        flags | FLAGS_EVENT_PRIORITY,
    )
}

/// `int InterruptAttachThread(int intr, unsigned flags)`: an attach with
/// neither handler nor event, for a thread that waits in `InterruptWait()`.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachThread(intr: c_int, flags: c_uint) -> c_int {
    InterruptAttach(intr, core::ptr::null(), core::ptr::null(), 0, flags)
}

/// `InterruptAttachThread()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachThread_r(intr: c_int, flags: c_uint) -> c_int {
    InterruptAttach_r(intr, core::ptr::null(), core::ptr::null(), 0, flags)
}

/// `int InterruptAttachArray(int intr, handler, const void *area, int size, unsigned flags)`:
/// an attach whose handler returns an array of events.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachArray(
    intr: c_int,
    handler: *const c_void,
    area: *const c_void,
    size: c_int,
    flags: c_uint,
) -> c_int {
    InterruptAttach(intr, handler, area, size, flags | FLAGS_ARRAY)
}

/// `InterruptAttachArray()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptAttachArray_r(
    intr: c_int,
    handler: *const c_void,
    area: *const c_void,
    size: c_int,
    flags: c_uint,
) -> c_int {
    InterruptAttach_r(intr, handler, area, size, flags | FLAGS_ARRAY)
}

/// `int InterruptHookTrace(handler, unsigned flags)`: a handler for the
/// kernel's trace buffer.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptHookTrace(handler: *const c_void, flags: c_uint) -> c_int {
    InterruptAttach(
        HOOK_TRACE,
        handler,
        core::ptr::null(),
        0,
        flags | FLAGS_PROCESS,
    )
}

/// `int InterruptHookIdle2(handler, unsigned flags)`: a handler the kernel
/// calls when a CPU goes idle.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptHookIdle2(handler: *const c_void, flags: c_uint) -> c_int {
    InterruptAttach(
        HOOK_IDLE2,
        handler,
        core::ptr::null(),
        0,
        flags | FLAGS_PROCESS,
    )
}

/// The handler `InterruptHookIdle()` was given.
static IDLE: core::sync::atomic::AtomicPtr<c_void> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

/// The idle hook's handler for the older interface: on CPU 0, the
/// handler with the time from the kernel's entry, and the system page's
/// time section (its offset at 0x14).
unsafe extern "C" fn idle_shim(cpu: c_int, syspage: *const u8, entry: *const u8) -> *const c_void {
    let h = IDLE.load(core::sync::atomic::Ordering::Acquire);
    if cpu != 0 || h.is_null() {
        return core::ptr::null();
    }
    // SAFETY: the kernel's arguments to an idle hook.
    let t = unsafe { entry.wrapping_add(0x10).cast::<u64>().read() };
    // SAFETY: as above.
    let mut t = t.wrapping_add(unsafe { entry.wrapping_add(0x20).cast::<u64>().read() });
    // SAFETY: as above: the system page's section table.
    let qtime = syspage.wrapping_add(usize::from(unsafe {
        syspage.wrapping_add(0x14).cast::<u16>().read()
    }));
    // SAFETY: a handler of the old form, given by InterruptHookIdle().
    let f = unsafe {
        core::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut u64, *const u8)>(h)
    };
    // SAFETY: as above.
    unsafe { f(&mut t, qtime) };
    core::ptr::null()
}

/// `int InterruptHookIdle(void (*handler)(_Uint64t *, struct qtime_entry
/// *), unsigned flags)`: the older idle hook, adapted to the kernel's.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptHookIdle(handler: *mut c_void, flags: c_uint) -> c_int {
    let old = IDLE.swap(handler, core::sync::atomic::Ordering::AcqRel);
    let r = InterruptAttach(
        HOOK_IDLE2,
        idle_shim as *const c_void,
        core::ptr::null(),
        0,
        flags | FLAGS_PROCESS,
    );
    if r < 0 {
        IDLE.store(old, core::sync::atomic::Ordering::Release);
    }
    r
}

/// `int InterruptHookOverdriveEvent(const struct sigevent *event, unsigned flags)`:
/// an event for the kernel's overdrive hook.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptHookOverdriveEvent(event: *const c_void, flags: c_uint) -> c_int {
    InterruptAttach(
        HOOK_OVERDRIVE,
        core::ptr::null(),
        event,
        SIGEVENT_SIZE,
        flags | FLAGS_PROCESS,
    )
}

/// `_NTO_INTR_WAIT_FLAGS_FAST` and `_NTO_INTR_WAIT_FLAGS_UNBLOCK`: the
/// fast wait, and whether an unblock may end it.
const WAIT_FAST: c_int = 1;
const WAIT_UNBLOCK: c_int = 2;

/// `int InterruptWait(int flags, const _Uint64t *timeout)`: waits for an
/// interrupt the thread attached. The fast wait takes a timeout; the
/// ordinary one does not (`ENOTSUP`).
#[unsafe(no_mangle)]
pub extern "C" fn InterruptWait(flags: c_int, timeout: *const u64) -> c_int {
    let r = InterruptWait_r(flags, timeout);
    if r == 0 {
        return 0;
    }
    crate::tls::set_errno(r);
    -1
}

/// `InterruptWait()`, with the `_r` convention.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptWait_r(flags: c_int, timeout: *const u64) -> c_int {
    if flags & WAIT_FAST != 0 {
        return __MuonCallInterruptWait_r(c_int::from(flags & WAIT_UNBLOCK != 0), timeout);
    }
    if !timeout.is_null() {
        return 48;
    }
    __InterruptWait_r(flags, timeout)
}

/// An `intr_capture()` handler: its area, interrupt, attachment and
/// function.
#[repr(C)]
struct Capture {
    area: *mut c_void,
    intr: c_int,
    id: c_int,
    handler: Option<unsafe extern "C" fn(*mut c_void) -> c_int>,
}

/// The handlers, as many as QNX's library keeps. Neither library locks
/// them.
static mut CAPTURES: [Capture; 32] = [const {
    Capture {
        area: core::ptr::null_mut(),
        intr: 0,
        id: 0,
        handler: None,
    }
}; 32];

/// `SIGEV_INTR`: the event that ends an `InterruptWait()`.
static INTR_EVENT: [u64; 4] = [6, 0, 0, 0];

/// The kernel's handler for a capture: its function with its area, then
/// the event if it returned 0.
unsafe extern "C" fn capture_stub(c: *const c_void, _id: c_int) -> *const c_void {
    // SAFETY: the capture this was attached with.
    let c = unsafe { &*c.cast::<Capture>() };
    // SAFETY: the caller of intr_capture()'s function and area.
    match c.handler.map(|f| unsafe { f(c.area) }) {
        Some(0) => (&raw const INTR_EVENT).cast(),
        _ => core::ptr::null(),
    }
}

fn captures() -> &'static mut [Capture; 32] {
    let p = &raw mut CAPTURES;
    // SAFETY: as above: QNX's library does not lock them either.
    unsafe { &mut *p }
}

/// `int intr_capture(intr_t intr, int (*handler)(volatile void *area),
/// volatile void *area, size_t areasize)`: calls `handler(area)` at each
/// interrupt `intr`, which wakes `intr_timed_wait()` when it returns 0.
/// 0, `EAGAIN` with 32 already, or the error.
#[unsafe(no_mangle)]
pub extern "C" fn intr_capture(
    intr: c_int,
    handler: unsafe extern "C" fn(*mut c_void) -> c_int,
    area: *mut c_void,
    _size: usize,
) -> c_int {
    let Some(c) = captures().iter_mut().find(|c| c.handler.is_none()) else {
        return 11;
    };
    *c = Capture {
        area,
        intr,
        id: 0,
        handler: Some(handler),
    };
    let r = InterruptAttach_r(
        intr,
        capture_stub as *const c_void,
        (&raw const *c).cast(),
        24,
        0,
    );
    if r < 0 {
        c.handler = None;
        return -r;
    }
    c.id = r;
    0
}

/// `int intr_release(intr_t intr, int (*handler)(volatile void *))`:
/// detaches what `intr_capture()` attached; `EINVAL` if nothing.
#[unsafe(no_mangle)]
pub extern "C" fn intr_release(
    intr: c_int,
    handler: unsafe extern "C" fn(*mut c_void) -> c_int,
) -> c_int {
    let found = captures()
        .iter_mut()
        .find(|c| c.intr == intr && c.handler.is_some_and(|h| core::ptr::fn_addr_eq(h, handler)));
    let Some(c) = found else {
        return 22;
    };
    let r = InterruptDetach_r(c.id);
    c.handler = None;
    r
}

/// `int intr_lock(intr_t intr)`: masks interrupt `intr`; 0, or
/// `EACCES`.
#[unsafe(no_mangle)]
pub extern "C" fn intr_lock(intr: c_int) -> c_int {
    if InterruptMask(intr, -1) == -1 { 13 } else { 0 }
}

/// `int intr_unlock(intr_t intr)`: unmasks it.
#[unsafe(no_mangle)]
pub extern "C" fn intr_unlock(intr: c_int) -> c_int {
    if InterruptUnmask(intr, -1) == -1 {
        13
    } else {
        0
    }
}

/// `int intr_timed_wait(int flags, const struct timespec *timeout)`:
/// `InterruptWait()`, ended after `timeout` if not null.
///
/// # Safety
///
/// `timeout` is null or a `struct timespec`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn intr_timed_wait(flags: c_int, timeout: *const [i64; 2]) -> c_int {
    // SAFETY: the caller's guarantee.
    if let Some(&[s, ns]) = unsafe { timeout.as_ref() } {
        let t = (s as u64)
            .saturating_mul(1_000_000_000)
            .saturating_add(ns as u64);
        // _NTO_TIMEOUT_INTR, relative.
        let r =
            super::timer::TimerTimeout_r(0, 1 << 16, core::ptr::null(), &t, core::ptr::null_mut());
        if r < 0 {
            return -r;
        }
    }
    InterruptWait_r(flags, core::ptr::null())
}

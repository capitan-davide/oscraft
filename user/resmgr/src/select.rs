//! `select_attach()`: the dispatch layer watches descriptors for its
//! server. Each one is armed with `ionotify()` and a pulse to the handle's
//! own channel, whose value is the descriptor; the pulse's handler calls
//! the server's function with the conditions that hold, then arms the
//! descriptor again.

use core::ffi::{c_int, c_uint, c_void};
use core::ptr::null_mut;

use kercall::neutrino::msg::{MsgRegisterEvent, MsgSendPulse, MsgUnregisterEvent};
use crate::compat::ionotify;
use proto::event::{SIGEV_FLAG_HANDLE, SIGEV_PULSE, Sigevent};
use proto::io::{NOTIFY_ACTION_POLL, NOTIFY_ACTION_POLLARM};

use crate::abi::*;
use crate::dispatch::{Dispatch, fail, message_connect, pulse_attach};
use crate::mem::{calloc, free};

type SelectFn = unsafe extern "C" fn(*mut Context, c_int, c_uint, *mut c_void) -> c_int;

/// `SELECT_FLAG_*`: the conditions (`_NOTIFY_COND_*`), and whether to arm
/// again after the function.
const CONDITIONS: c_uint = 0x7000_0000;
const NOREARM: c_uint = 0x4;
/// `MSG_FLAG_ALLOC_PULSE`, `MSG_FLAG_SIDE_CHANNEL`.
const ALLOC_PULSE: c_int = 0x2;
const SIDE_CHANNEL: c_int = 0x200;

/// A handle's watched descriptors: the pulse code and connection they
/// notify, and the list.
pub struct Selects {
    code: c_int,
    coid: c_int,
    entries: *mut Entry,
}

struct Entry {
    next: *mut Entry,
    fd: c_int,
    /// Its pulse, registered, as servers deliver only those.
    event: c_int,
    flags: c_uint,
    func: SelectFn,
    handle: *mut c_void,
}

/// The handle's descriptors, set up the first time.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
unsafe fn selects<'a>(dpp: *mut Dispatch) -> Option<&'a mut Selects> {
    // SAFETY: the caller's guarantee.
    let d = unsafe { &mut *dpp };
    if d.select.is_null() {
        // SAFETY: the handle; on_pulse() takes it.
        let code = unsafe { pulse_attach(dpp, ALLOC_PULSE, 0, Some(on_pulse), dpp.cast()) };
        // SAFETY: as above.
        let coid = unsafe { message_connect(dpp, SIDE_CHANNEL) };
        let s = calloc::<Selects>();
        if code < 0 || coid < 0 || s.is_null() {
            return None;
        }
        // SAFETY: room for it.
        unsafe {
            s.write(Selects {
                code,
                coid,
                entries: null_mut(),
            })
        };
        d.select = s;
    }
    // SAFETY: set above, or before.
    unsafe { d.select.as_mut() }
}

/// The entry of `fd`.
fn find(s: &Selects, fd: c_int) -> Option<&Entry> {
    let mut e = s.entries;
    // SAFETY: the list's entries.
    while let Some(en) = unsafe { e.as_ref() } {
        if en.fd == fd {
            return Some(en);
        }
        e = en.next;
    }
    None
}

/// Registers the pulse for `fd`: its handle, or -1.
fn register(s: &Selects, fd: c_int) -> c_int {
    let mut event = Sigevent {
        notify: SIGEV_PULSE,
        signo: s.coid,
        value: fd as u64,
        code: s.code as i16,
        priority: -1,
        ..Sigevent::default()
    };
    // SAFETY: a struct sigevent, for any server.
    if unsafe { MsgRegisterEvent((&raw mut event).cast(), -1) } == -1 {
        return -1;
    }
    event.signo
}

/// The registered pulse of `handle`, as `ionotify()` takes it.
fn registered(handle: c_int) -> Sigevent {
    Sigevent {
        notify: SIGEV_PULSE | SIGEV_FLAG_HANDLE,
        signo: handle,
        ..Sigevent::default()
    }
}

/// Arms `e`'s descriptor; if a condition holds already, the pulse is sent
/// at once.
fn arm(s: &Selects, e: &Entry) -> c_int {
    let event = registered(e.event);
    // SAFETY: a struct sigevent.
    let r = unsafe {
        ionotify(
            e.fd,
            NOTIFY_ACTION_POLLARM,
            (e.flags & CONDITIONS) as c_int,
            (&raw const event).cast(),
        )
    };
    if r > 0 {
        MsgSendPulse(s.coid, -1, s.code, e.fd as _);
    }
    r.min(0)
}

/// The pulse: the descriptor's conditions, the server's function, and
/// the descriptor armed again unless `SELECT_FLAG_NOREARM`.
unsafe extern "C" fn on_pulse(
    ctp: *mut Context,
    _code: c_int,
    _flags: c_uint,
    dpp: *mut c_void,
) -> c_int {
    // SAFETY: the dispatch layer's context.
    let msg = unsafe { (*ctp).msg };
    // SAFETY: it holds the pulse; its value at 8 is the descriptor.
    let fd = unsafe { msg.wrapping_add(8).cast::<c_int>().read_unaligned() };
    // SAFETY: the handle this was attached with.
    let Some(s) = (unsafe { selects(dpp.cast()) }) else {
        return 0;
    };
    let Some(e) = find(s, fd) else { return 0 };
    // SAFETY: a poll, without an event.
    let ready = unsafe {
        ionotify(
            fd,
            NOTIFY_ACTION_POLL,
            (e.flags & CONDITIONS) as c_int,
            core::ptr::null(),
        )
    };
    let (func, handle, flags) = (e.func, e.handle, e.flags);
    if ready > 0 {
        // SAFETY: the server's function, with its handle.
        unsafe { func(ctp, fd, ready as c_uint, handle) };
    }
    if flags & NOREARM == 0
        && let Some(e) = find(s, fd)
    {
        arm(s, e);
    }
    0
}

/// Watches `fd` for the conditions in `flags` (`SELECT_FLAG_READ`,
/// `_WRITE`, `_EXCEPT`), calling `func(ctp, fd, conditions, handle)` from
/// the dispatch loop when one holds. 0, or -1 with `errno`.
///
/// # Safety
///
/// `dpp` is a dispatch handle; `func` takes `handle`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn select_attach(
    dpp: *mut Dispatch,
    _attr: *const c_void,
    fd: c_int,
    flags: c_uint,
    func: SelectFn,
    handle: *mut c_void,
) -> c_int {
    if flags & CONDITIONS == 0 {
        return fail(proto::errno::EINVAL);
    }
    // SAFETY: the caller's guarantee.
    let Some(s) = (unsafe { selects(dpp) }) else {
        return fail(proto::errno::ENOMEM);
    };
    let event = register(s, fd);
    let e = calloc::<Entry>();
    if e.is_null() || event == -1 {
        return fail(proto::errno::ENOMEM);
    }
    let entry = Entry {
        next: s.entries,
        fd,
        event,
        flags,
        func,
        handle,
    };
    // SAFETY: room for it; then the list's head.
    unsafe { e.write(entry) };
    s.entries = e;
    // SAFETY: as above.
    if arm(s, unsafe { &*e }) == -1 { -1 } else { 0 }
}

/// Stops watching `fd`: 0, or -1 if it was not watched.
///
/// # Safety
///
/// `dpp` is a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn select_detach(dpp: *mut Dispatch, fd: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let list = unsafe { (*dpp).select };
    // SAFETY: null, or the handle's descriptors.
    let Some(s) = (unsafe { list.as_mut() }) else {
        return -1;
    };
    let mut link = &raw mut s.entries;
    loop {
        // SAFETY: the list's links.
        let cur = unsafe { *link };
        // SAFETY: as above.
        let Some(e) = (unsafe { cur.as_mut() }) else {
            return -1;
        };
        if e.fd == fd {
            let event = registered(e.event);
            // SAFETY: the registered event.
            unsafe { MsgUnregisterEvent((&raw const event).cast()) };
            // SAFETY: as above; the entry from calloc().
            unsafe { *link = e.next };
            // SAFETY: as above.
            unsafe { free(cur.cast()) };
            return 0;
        }
        link = &raw mut e.next;
    }
}

/// The descriptors of the context's handle, if it watches any.
///
/// # Safety
///
/// `ctp` is a context.
unsafe fn of<'a>(ctp: *mut Context) -> Option<&'a Selects> {
    // SAFETY: the caller's guarantee.
    let dpp = unsafe { (*ctp).dpp };
    // SAFETY: null, or the context's handle.
    let d = unsafe { dpp.as_ref() }?;
    // SAFETY: null, or its descriptors.
    unsafe { d.select.as_ref() }
}

/// Writes `v` through `p` unless it is null.
///
/// # Safety
///
/// `p` is null or writable.
unsafe fn put<T>(p: *mut T, v: T) {
    if !p.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { p.write(v) };
    }
}

/// For a context that holds a select pulse: its descriptor, conditions,
/// function and handle. 0, or -1 if it holds none.
///
/// # Safety
///
/// `ctp` is a context; the others are null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn select_query(
    ctp: *mut Context,
    fd: *mut c_int,
    flags: *mut c_uint,
    func: *mut Option<SelectFn>,
    handle: *mut *mut c_void,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let Some(s) = (unsafe { of(ctp) }) else {
        return -1;
    };
    // SAFETY: as above.
    let msg = unsafe { (*ctp).msg };
    // SAFETY: the context's message: a pulse's code at 4, its value at 8.
    let code = unsafe { msg.wrapping_add(4).read() } as i8;
    // SAFETY: as above.
    let value = unsafe { msg.wrapping_add(8).cast::<c_int>().read_unaligned() };
    if c_int::from(code) != s.code {
        return -1;
    }
    let Some(e) = find(s, value) else {
        return -1;
    };
    // SAFETY: as above: each null or writable.
    unsafe { put(fd, e.fd) };
    // SAFETY: as above.
    unsafe { put(flags, e.flags) };
    // SAFETY: as above.
    unsafe { put(func, Some(e.func)) };
    // SAFETY: as above.
    unsafe { put(handle, e.handle) };
    0
}

/// Arms `fd` again, for a server that dispatches itself: the context.
///
/// # Safety
///
/// `ctp` is a context of a dispatch handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn select_rearm(ctp: *mut Context, fd: c_int) -> *mut Context {
    // SAFETY: the caller's guarantee.
    if let Some(s) = unsafe { of(ctp) }
        && let Some(e) = find(s, fd)
    {
        arm(s, e);
    }
    ctp
}

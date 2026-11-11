//! `poll()`, `ppoll()`, `select()` and `pselect()`: as QNX's libc does
//! them, with `_IO_NOTIFY` messages (`ionotify()`). Each descriptor's
//! server is asked to poll the conditions and arm them if none holds
//! (`_NOTIFY_ACTION_POLLARM`) with an event registered for it
//! (`MsgRegisterEvent()`): a `SIGSELECT` signal to the calling thread,
//! which `SignalWaitinfo()` takes, with `TimerTimeout()` as the limit.
//! Then each server is asked once more (`_NOTIFY_ACTION_POLL`), which
//! reports the conditions and disarms, and the events are unregistered.
//! A server that knows no notification (a file, `/dev/null`) is always
//! ready for what was asked, as a regular file is on FreeBSD.
//!
//! FreeBSD's `pollfd` and `fd_set` are the caller's; QNX's conditions
//! (`<sys/iomgr.h>`: INPUT, OUTPUT, OBAND, and the extended RDNORM &
//! co., ERR, HUP, NVAL) are the message's.

use core::ffi::c_int;
use core::ptr::{null, null_mut};

use proto::errno::{EAGAIN, EBADF, EINVAL, ENOSYS, ENOTSUP, ETIMEDOUT};
use proto::event::{SIGEV_FLAG_HANDLE, SIGEV_SIGNAL_THREAD, SI_NOTIFY, Sigevent};
use proto::io::{IO_NOTIFY, NOTIFY_ACTION_POLL, NOTIFY_ACTION_POLLARM, NOTIFY_COND_EXTEN, Notify};

use crate::errno::fail;
use crate::kercall::{self, Iov, msg_sendv};
use crate::signal::SigSet;

/// FreeBSD's `struct pollfd` and its events (`<poll.h>`).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pollfd {
    pub fd: c_int,
    pub events: i16,
    pub revents: i16,
}

const POLLIN: i16 = 0x1;
const POLLPRI: i16 = 0x2;
const POLLOUT: i16 = 0x4;
const POLLERR: i16 = 0x8;
const POLLHUP: i16 = 0x10;
const POLLNVAL: i16 = 0x20;
const POLLRDNORM: i16 = 0x40;
const POLLWRNORM: i16 = POLLOUT;
const POLLRDBAND: i16 = 0x80;
const POLLWRBAND: i16 = 0x100;

/// QNX's conditions.
const COND_OBAND: u32 = 0x4000_0000;
const COND_OUTPUT: u32 = 0x2000_0000;
const COND_INPUT: u32 = 0x1000_0000;
const CONDE_RDNORM: u32 = 0x1;
const CONDE_WRNORM: u32 = 0x2;
const CONDE_RDBAND: u32 = 0x4;
const CONDE_PRI: u32 = 0x8;
const CONDE_WRBAND: u32 = 0x10;
const CONDE_ERR: u32 = 0x20;
const CONDE_HUP: u32 = 0x40;
const CONDE_NVAL: u32 = 0x1000;

/// `SIGSELECT`: QNX's signal for `poll()`, always blocked and queued.
const SIGSELECT: u32 = 57;
/// FreeBSD's `FD_SETSIZE`, in `fd_mask`s of 64 bits.
const FD_SETSIZE: usize = 1024;
const FD_WORDS: usize = FD_SETSIZE / 64;

/// The conditions below the top four bits, in the extended form.
const COND_DATA: u32 = 0x0fff_ffff;

/// QNX's conditions for FreeBSD's events, as QNX's libc asks: the basic
/// ones in the top bits and, for a server that takes the extended form
/// (`_NOTIFY_COND_EXTEN`: bands, errors and hangups as their own
/// conditions), the rest below them. That form also carries the whole
/// `pollfd` array to a QNX server; ours ask one descriptor at a time, so
/// QNX's servers get the plain form, and report a hangup or an error as
/// input or output, which a read or write then finds.
fn conditions(events: i16, extended: bool) -> u32 {
    let mut cond = 0;
    let mut exten = CONDE_ERR | CONDE_HUP | CONDE_NVAL;
    if events & (POLLIN | POLLRDNORM) != 0 {
        cond |= COND_INPUT;
        exten |= CONDE_RDNORM;
    }
    if events & (POLLOUT | POLLWRNORM | POLLWRBAND) != 0 {
        cond |= COND_OUTPUT;
        exten |= CONDE_WRNORM;
        if events & POLLWRBAND != 0 {
            exten |= CONDE_WRBAND;
        }
    }
    if events & (POLLPRI | POLLRDBAND) != 0 {
        cond |= COND_OBAND;
        exten |= CONDE_RDBAND | CONDE_PRI;
    }
    if extended { cond | exten | NOTIFY_COND_EXTEN } else { cond }
}

/// FreeBSD's events for QNX's conditions that hold, among `events`.
fn revents(events: i16, held: u32) -> i16 {
    let exten = if held & NOTIFY_COND_EXTEN != 0 { held & COND_DATA } else { 0 };
    let mut r = 0;
    if held & COND_INPUT != 0 || exten & CONDE_RDNORM != 0 {
        r |= POLLIN | POLLRDNORM;
    }
    if held & COND_OUTPUT != 0 || exten & CONDE_WRNORM != 0 {
        r |= POLLOUT | POLLWRNORM;
    }
    if exten & CONDE_WRBAND != 0 {
        r |= POLLWRBAND;
    }
    if held & COND_OBAND != 0 || exten & (CONDE_RDBAND | CONDE_PRI) != 0 {
        r |= POLLPRI | POLLRDBAND;
    }
    r &= events;
    if exten & CONDE_ERR != 0 {
        r |= POLLERR;
    }
    if exten & CONDE_HUP != 0 {
        r |= POLLHUP;
    }
    if exten & CONDE_NVAL != 0 {
        r |= POLLNVAL;
    }
    r
}

/// One `_IO_NOTIFY` on `fd`: the conditions that hold.
///
/// # Safety
///
/// `event` is a registered event's handle form, or null.
unsafe fn notify(fd: c_int, action: i32, cond: u32, event: Option<&Sigevent>) -> Result<u32, i32> {
    let mut msg = Notify {
        r#type: IO_NOTIFY,
        combine_len: size_of::<Notify>() as u16,
        action,
        flags: cond,
        ..Notify::default()
    };
    if let Some(ev) = event {
        msg.event = *ev;
    }
    let mut reply = [0u8; size_of::<Notify>()];
    let send = [Iov::raw((&raw const msg).cast(), size_of::<Notify>())];
    let recv = [Iov::raw(reply.as_mut_ptr().cast(), reply.len())];
    // SAFETY: both are on this stack.
    unsafe { msg_sendv(fd, &send, &recv) }?;
    // The reply's conditions, at 4.
    Ok(u32::from_le_bytes([reply[4], reply[5], reply[6], reply[7]]))
}

/// What one descriptor's arming found: ready now, or armed with its
/// event's handle, or a server that cannot say (always ready).
#[derive(Clone, Copy, PartialEq)]
enum Armed {
    Ready,
    Waiting(u32),
    Plain,
    Invalid,
}

/// The wait: how long, in nanoseconds (None: for ever), and the mask
/// meanwhile.
struct Wait<'a> {
    ns: Option<u64>,
    mask: Option<&'a [u32; 4]>,
}

/// `poll()` proper: fills `revents`, returns how many descriptors have
/// any, or QNX's error.
///
/// # Safety
///
/// `fds` holds `nfds` entries.
unsafe fn poll(fds: *mut Pollfd, nfds: usize, wait: &Wait<'_>) -> Result<c_int, i32> {
    let handles = crate::mem::scratch(nfds.max(1) * size_of::<Armed>())?.cast::<Armed>();
    // SAFETY: the mapping holds nfds of them.
    let armed = unsafe { core::slice::from_raw_parts_mut(handles, nfds) };
    // SAFETY: the caller's guarantee.
    let fds = unsafe { core::slice::from_raw_parts_mut(fds, nfds) };
    let r = arm_and_wait(fds, armed, wait);
    crate::mem::unscratch(handles.cast(), nfds.max(1) * size_of::<Armed>());
    r
}

fn arm_and_wait(fds: &mut [Pollfd], armed: &mut [Armed], wait: &Wait<'_>) -> Result<c_int, i32> {
    let mut ready = 0;
    let set: [u32; 4] = [0, 1 << (SIGSELECT - 33), 0, 0];
    for (pf, slot) in fds.iter_mut().zip(armed.iter_mut()) {
        pf.revents = 0;
        if pf.fd < 0 {
            *slot = Armed::Plain;
            continue;
        }
        let cond = conditions(pf.events, crate::fd::native(pf.fd));
        // The event: SIGSELECT to this thread, registered for the server.
        let ev = Sigevent {
            notify: SIGEV_SIGNAL_THREAD,
            signo: SIGSELECT as i32,
            code: SI_NOTIFY,
            value: pf.fd as u32 as u64,
            ..Sigevent::default()
        };
        let mut handle = 0u32;
        // SAFETY: the event and the handle are on this stack.
        *slot = match unsafe { kercall::msg_register_event(&ev, pf.fd, &mut handle) } {
            Err(EBADF) => Armed::Invalid,
            Err(e) => {
                disarm(fds, armed, &set);
                return Err(e);
            }
            Ok(()) => {
                let by_handle = Sigevent {
                    notify: SIGEV_FLAG_HANDLE,
                    signo: handle as i32,
                    ..Sigevent::default()
                };
                // SAFETY: a registered event's handle.
                match unsafe { notify(pf.fd, NOTIFY_ACTION_POLLARM, cond, Some(&by_handle)) } {
                    Ok(0) => Armed::Waiting(handle),
                    Ok(held) => {
                        pf.revents = revents(pf.events, held);
                        unregister(handle);
                        Armed::Ready
                    }
                    Err(EBADF) => {
                        unregister(handle);
                        Armed::Invalid
                    }
                    Err(ENOSYS | ENOTSUP | EINVAL) => {
                        unregister(handle);
                        Armed::Plain
                    }
                    Err(e) => {
                        unregister(handle);
                        disarm(fds, armed, &set);
                        return Err(e);
                    }
                }
            }
        };
        match *slot {
            Armed::Invalid => pf.revents = POLLNVAL,
            Armed::Plain if pf.fd >= 0 => pf.revents = pf.events & (POLLIN | POLLRDNORM | POLLOUT | POLLWRNORM),
            _ => {}
        }
        if pf.revents != 0 {
            ready += 1;
        }
    }
    if ready == 0 && wait.ns != Some(0) {
        if let Some(ns) = wait.ns {
            let _ = kercall::timer_timeout(0, kercall::TIMEOUT_SIGWAITINFO, ns);
        }
        // SAFETY: the set and the mask are valid for the call.
        match unsafe { kercall::signal_waitinfo(&set, null_mut(), wait.mask.map_or(null(), |m| m)) } {
            Ok(_) | Err(ETIMEDOUT) | Err(EAGAIN) => {}
            Err(e) => {
                disarm(fds, armed, &set);
                return Err(e);
            }
        }
    }
    let r = disarm(fds, armed, &set);
    Ok(r)
}

/// Asks each armed descriptor's server what holds (which disarms it),
/// unregisters the events and drains the signals left: how many
/// descriptors have events.
fn disarm(fds: &mut [Pollfd], armed: &mut [Armed], set: &[u32; 4]) -> c_int {
    let mut ready = 0;
    for (pf, slot) in fds.iter_mut().zip(armed.iter_mut()) {
        if let Armed::Waiting(handle) = *slot {
            let cond = conditions(pf.events, crate::fd::native(pf.fd));
            // SAFETY: no event: a poll.
            if let Ok(held) = unsafe { notify(pf.fd, NOTIFY_ACTION_POLL, cond, None) } {
                pf.revents = revents(pf.events, held);
            }
            unregister(handle);
            *slot = Armed::Ready;
        }
        if pf.revents != 0 {
            ready += 1;
        }
    }
    // The signals of the events that fired, so that the next poll doesn't
    // wake at once (a passed deadline ends the wait at once).
    loop {
        let _ = kercall::timer_timeout(0, kercall::TIMEOUT_SIGWAITINFO, 0);
        // SAFETY: the set is on this stack; no info, no mask.
        if unsafe { kercall::signal_waitinfo(set, null_mut(), null()) }.is_err() {
            break;
        }
    }
    ready
}

fn unregister(handle: u32) {
    let mut h = handle;
    // SAFETY: a null event unregisters the handle.
    let _ = unsafe { kercall::msg_register_event(null(), 0, &mut h) };
}

/// `poll(fds, nfds, timeout)`: `timeout` in milliseconds, negative for
/// ever.
///
/// # Safety
///
/// `fds` holds `nfds` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_poll(fds: *mut Pollfd, nfds: u32, timeout: c_int) -> c_int {
    let wait = Wait {
        ns: if timeout < 0 { None } else { Some(timeout as u64 * 1_000_000) },
        mask: None,
    };
    // SAFETY: the caller's guarantee.
    unsafe { poll(fds, nfds as usize, &wait) }.unwrap_or_else(|e| fail(e) as c_int)
}

/// `ppoll(fds, nfds, timeout, sigmask)`.
///
/// # Safety
///
/// `fds` holds `nfds` entries; `timeout` is a timespec or null; `sigmask`
/// a signal set or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_ppoll(fds: *mut Pollfd, nfds: u32, timeout: *const [i64; 2], sigmask: *const SigSet) -> c_int {
    let ns = if timeout.is_null() {
        None
    } else {
        // SAFETY: the caller's guarantee.
        let [s, n] = unsafe { *timeout };
        if s < 0 || !(0..1_000_000_000).contains(&n) {
            return fail(EINVAL) as c_int;
        }
        Some(s as u64 * 1_000_000_000 + n as u64)
    };
    // SAFETY: as above.
    let qmask = (!sigmask.is_null()).then(|| unsafe { *sigmask });
    let wait = Wait { ns, mask: qmask.as_ref() };
    // SAFETY: the caller's guarantee.
    unsafe { poll(fds, nfds as usize, &wait) }.unwrap_or_else(|e| fail(e) as c_int)
}

/// `select()`'s sets as `pollfd`s, and back.
///
/// # Safety
///
/// Each set is null or an `fd_set`; `nfds` is at most `FD_SETSIZE`.
unsafe fn select(nfds: c_int, r: *mut [u64; FD_WORDS], w: *mut [u64; FD_WORDS], e: *mut [u64; FD_WORDS], wait: &Wait<'_>) -> Result<c_int, i32> {
    if !(0..=FD_SETSIZE as c_int).contains(&nfds) {
        return Err(EINVAL);
    }
    let bit = |set: *mut [u64; FD_WORDS], fd: usize| -> bool {
        // SAFETY: the caller's guarantee; fd < FD_SETSIZE.
        !set.is_null() && unsafe { (*set)[fd / 64] } & (1 << (fd % 64)) != 0
    };
    let fds_size = (nfds as usize).max(1) * size_of::<Pollfd>();
    let fds = crate::mem::scratch(fds_size)?.cast::<Pollfd>();
    let mut count = 0usize;
    for fd in 0..nfds as usize {
        let mut events = 0;
        if bit(r, fd) {
            events |= POLLIN;
        }
        if bit(w, fd) {
            events |= POLLOUT;
        }
        if bit(e, fd) {
            events |= POLLPRI;
        }
        if events != 0 {
            // SAFETY: in the mapping, count < nfds.
            unsafe { fds.add(count).write(Pollfd { fd: fd as c_int, events, revents: 0 }) };
            count += 1;
        }
    }
    // SAFETY: the mapping holds `count` entries.
    let polled = unsafe { poll(fds, count, wait) };
    let result = polled.and_then(|_| {
        let mut n = 0;
        let clear = |set: *mut [u64; FD_WORDS]| {
            if !set.is_null() {
                // SAFETY: the caller's guarantee.
                unsafe { (*set) = [0; FD_WORDS] };
            }
        };
        clear(r);
        clear(w);
        clear(e);
        for i in 0..count {
            // SAFETY: as above.
            let pf = unsafe { fds.add(i).read() };
            if pf.revents & POLLNVAL != 0 {
                return Err(EBADF);
            }
            let fd = pf.fd as usize;
            let mut mark = |set: *mut [u64; FD_WORDS], on: bool| {
                if on && !set.is_null() {
                    // SAFETY: the caller's guarantee.
                    unsafe { (*set)[fd / 64] |= 1 << (fd % 64) };
                    n += 1;
                }
            };
            mark(r, pf.events & POLLIN != 0 && pf.revents & (POLLIN | POLLHUP | POLLERR) != 0);
            mark(w, pf.events & POLLOUT != 0 && pf.revents & (POLLOUT | POLLERR) != 0);
            mark(e, pf.events & POLLPRI != 0 && pf.revents & POLLPRI != 0);
        }
        Ok(n)
    });
    crate::mem::unscratch(fds.cast(), fds_size);
    result
}

/// `select(nfds, readfds, writefds, exceptfds, timeout)`: `timeout` a
/// timeval or null.
///
/// # Safety
///
/// As `select()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_select(nfds: c_int, r: *mut [u64; FD_WORDS], w: *mut [u64; FD_WORDS], e: *mut [u64; FD_WORDS], timeout: *const [i64; 2]) -> c_int {
    let ns = if timeout.is_null() {
        None
    } else {
        // SAFETY: the caller's guarantee.
        let [s, us] = unsafe { *timeout };
        if s < 0 || !(0..1_000_000).contains(&us) {
            return fail(EINVAL) as c_int;
        }
        Some(s as u64 * 1_000_000_000 + us as u64 * 1000)
    };
    let wait = Wait { ns, mask: None };
    // SAFETY: the caller's guarantee.
    unsafe { select(nfds, r, w, e, &wait) }.unwrap_or_else(|e| fail(e) as c_int)
}

/// `pselect(nfds, readfds, writefds, exceptfds, timeout, sigmask)`:
/// `timeout` a timespec or null.
///
/// # Safety
///
/// As `pselect()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_pselect(nfds: c_int, r: *mut [u64; FD_WORDS], w: *mut [u64; FD_WORDS], e: *mut [u64; FD_WORDS], timeout: *const [i64; 2], sigmask: *const SigSet) -> c_int {
    let ns = if timeout.is_null() {
        None
    } else {
        // SAFETY: the caller's guarantee.
        let [s, n] = unsafe { *timeout };
        if s < 0 || !(0..1_000_000_000).contains(&n) {
            return fail(EINVAL) as c_int;
        }
        Some(s as u64 * 1_000_000_000 + n as u64)
    };
    // SAFETY: as above.
    let qmask = (!sigmask.is_null()).then(|| unsafe { *sigmask });
    let wait = Wait { ns, mask: qmask.as_ref() };
    // SAFETY: the caller's guarantee.
    unsafe { select(nfds, r, w, e, &wait) }.unwrap_or_else(|e| fail(e) as c_int)
}

const _: () = assert!(size_of::<Notify>() == 88);

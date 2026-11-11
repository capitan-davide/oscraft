//! `kqueue()` and `kevent()`, over `poll()`: the read and write filters
//! poll their descriptors, the timer filter keeps deadlines, the user
//! filter is triggered by its own changes. The queues live in a fixed
//! table here (libsys has no heap): a kqueue is a descriptor opened on
//! `/dev/null`, so that it is closed, inherited and numbered like one,
//! and its table slot is found by that number. The signal, vnode and
//! process filters have no counterpart yet: a change asking for one is
//! answered `EV_ERROR` with EINVAL, which programs take as "no kqueue
//! for this" and fall back on polling.

use core::ffi::c_int;
use core::ptr::null;
use core::sync::atomic::{AtomicBool, Ordering};

use proto::errno::{EBADF, EINVAL, EMFILE, ENOENT, ENOMEM};

use crate::errno::fail;
use crate::poll::{Pollfd, __sys_ppoll};

/// FreeBSD's `struct kevent`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Kevent {
    pub ident: usize,
    pub filter: i16,
    pub flags: u16,
    pub fflags: u32,
    pub data: i64,
    pub udata: usize,
    pub ext: [u64; 4],
}

const EVFILT_READ: i16 = -1;
const EVFILT_WRITE: i16 = -2;
const EVFILT_TIMER: i16 = -7;
const EVFILT_USER: i16 = -11;
const EV_ADD: u16 = 0x1;
const EV_DELETE: u16 = 0x2;
const EV_ENABLE: u16 = 0x4;
const EV_DISABLE: u16 = 0x8;
const EV_ONESHOT: u16 = 0x10;
const EV_CLEAR: u16 = 0x20;
const EV_RECEIPT: u16 = 0x40;
const EV_DISPATCH: u16 = 0x80;
const EV_EOF: u16 = 0x8000;
const EV_ERROR: u16 = 0x4000;
const NOTE_FFCTRLMASK: u32 = 0xc000_0000;
const NOTE_FFLAGSMASK: u32 = 0x00ff_ffff;
const NOTE_FFAND: u32 = 0x4000_0000;
const NOTE_FFOR: u32 = 0x8000_0000;
const NOTE_FFCOPY: u32 = 0xc000_0000;
const NOTE_TRIGGER: u32 = 0x0100_0000;
const NOTE_SECONDS: u32 = 0x1;
const NOTE_MSECONDS: u32 = 0x2;
const NOTE_USECONDS: u32 = 0x4;
const NOTE_NSECONDS: u32 = 0x8;
const NOTE_ABSTIME: u32 = 0x10;
const POLLIN: i16 = 0x1;
const POLLOUT: i16 = 0x4;
const POLLERR: i16 = 0x8;
const POLLHUP: i16 = 0x10;
const POLLNVAL: i16 = 0x20;
const NS: u64 = 1_000_000_000;

/// One registered event.
#[derive(Clone, Copy, Default)]
struct Entry {
    used: bool,
    enabled: bool,
    ev: Kevent,
    /// A timer's next expiry (monotonic ns) and period (0: once).
    deadline: u64,
    period: u64,
    /// A user event's trigger.
    triggered: bool,
}

const QUEUES: usize = 8;
const ENTRIES: usize = 64;

struct Queue {
    fd: c_int,
    entries: [Entry; ENTRIES],
}

struct Table {
    queues: [Queue; QUEUES],
}

static LOCK: AtomicBool = AtomicBool::new(false);
static mut TABLE: Table = Table {
    queues: [const {
        Queue {
            fd: -1,
            entries: [Entry {
                used: false,
                enabled: false,
                ev: Kevent {
                    ident: 0,
                    filter: 0,
                    flags: 0,
                    fflags: 0,
                    data: 0,
                    udata: 0,
                    ext: [0; 4],
                },
                deadline: 0,
                period: 0,
                triggered: false,
            }; ENTRIES],
        }
    }; QUEUES],
};

/// The table, under the lock while `Guard` lives.
struct Guard;

impl Guard {
    fn take() -> Guard {
        while LOCK.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            core::hint::spin_loop();
        }
        Guard
    }

    #[allow(clippy::mut_from_ref)]
    fn table(&self) -> &mut Table {
        // SAFETY: the lock is held while the guard lives: one reference
        // at a time.
        unsafe { &mut *(&raw mut TABLE) }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        LOCK.store(false, Ordering::Release);
    }
}

fn now_ns() -> u64 {
    crate::kercall::clock_time(2).unwrap_or(0)
}

/// `kqueue()`: a descriptor on `/dev/null` and a slot of the table.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_kqueue() -> c_int {
    __sys_kqueuex(0)
}

/// `kqueuex(flags)`: `KQUEUE_CLOEXEC` (1) is the descriptor's flag.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_kqueuex(flags: u32) -> c_int {
    const O_RDWR: c_int = 2;
    const O_CLOEXEC: c_int = 0x10_0000;
    let oflag = O_RDWR | if flags & 1 != 0 { O_CLOEXEC } else { 0 };
    // SAFETY: a literal path.
    let fd = unsafe { crate::fs::__sys_open(c"/dev/null".as_ptr(), oflag, 0) };
    if fd < 0 {
        return fd;
    }
    let g = Guard::take();
    let t = g.table();
    // A slot whose descriptor was closed and reused is this one's now.
    let slot = t.queues.iter().position(|q| q.fd == fd).or_else(|| t.queues.iter().position(|q| q.fd < 0));
    match slot {
        Some(i) => {
            t.queues[i].fd = fd;
            t.queues[i].entries = [Entry::default(); ENTRIES];
            fd
        }
        None => {
            drop(g);
            let _ = crate::fs::__sys_close(fd);
            fail(EMFILE) as c_int
        }
    }
}

/// The period or deadline a timer change asks for, in nanoseconds:
/// (deadline, period).
fn timer_of(data: i64, fflags: u32, now: u64) -> (u64, u64) {
    let unit = if fflags & NOTE_SECONDS != 0 {
        NS
    } else if fflags & NOTE_USECONDS != 0 {
        1000
    } else if fflags & NOTE_NSECONDS != 0 {
        1
    } else {
        let _ = NOTE_MSECONDS;
        1_000_000
    };
    let ns = data.max(0) as u64 * unit;
    if fflags & NOTE_ABSTIME != 0 {
        // An absolute time of the realtime clock: as an offset from now.
        let real = crate::kercall::clock_time(0).unwrap_or(0);
        (now + ns.saturating_sub(real), 0)
    } else {
        (now + ns, ns)
    }
}

/// Applies one change to queue `q`: `Ok`, or the errno for an `EV_ERROR`
/// event.
fn change(q: &mut Queue, c: &Kevent, now: u64) -> Result<(), c_int> {
    if !matches!(c.filter, EVFILT_READ | EVFILT_WRITE | EVFILT_TIMER | EVFILT_USER) {
        return Err(EINVAL);
    }
    let found = q.entries.iter().position(|e| e.used && e.ev.ident == c.ident && e.ev.filter == c.filter);
    if c.flags & EV_DELETE != 0 {
        let i = found.ok_or(ENOENT)?;
        q.entries[i] = Entry::default();
        return Ok(());
    }
    let i = match found {
        Some(i) => i,
        None if c.flags & EV_ADD != 0 => {
            let i = q.entries.iter().position(|e| !e.used).ok_or(ENOMEM)?;
            q.entries[i] = Entry {
                used: true,
                enabled: true,
                ev: Kevent { flags: c.flags & !(EV_ADD | EV_DELETE | EV_ENABLE | EV_DISABLE | EV_RECEIPT), ..*c },
                ..Entry::default()
            };
            i
        }
        None => return Err(ENOENT),
    };
    let e = &mut q.entries[i];
    if c.flags & EV_ADD != 0 {
        e.ev.udata = c.udata;
        e.ev.flags = c.flags & !(EV_ADD | EV_DELETE | EV_ENABLE | EV_DISABLE | EV_RECEIPT);
        match c.filter {
            EVFILT_TIMER => {
                (e.deadline, e.period) = timer_of(c.data, c.fflags, now);
                e.ev.fflags = c.fflags;
                e.ev.data = c.data;
            }
            EVFILT_USER => e.ev.fflags = c.fflags & NOTE_FFLAGSMASK,
            _ => e.ev.fflags = c.fflags,
        }
    }
    if c.filter == EVFILT_USER {
        let bits = c.fflags & NOTE_FFLAGSMASK;
        e.ev.fflags = match c.fflags & NOTE_FFCTRLMASK {
            NOTE_FFAND => e.ev.fflags & bits,
            NOTE_FFOR => e.ev.fflags | bits,
            NOTE_FFCOPY => bits,
            _ => e.ev.fflags,
        };
        if c.fflags & NOTE_TRIGGER != 0 {
            e.triggered = true;
        }
    }
    if c.flags & EV_ENABLE != 0 {
        e.enabled = true;
    }
    if c.flags & EV_DISABLE != 0 {
        e.enabled = false;
    }
    Ok(())
}

/// `kevent(kq, changelist, nchanges, eventlist, nevents, timeout)`.
///
/// # Safety
///
/// The lists hold their counts of events; `timeout` is a timespec or
/// null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_kevent(kq: c_int, changes: *const Kevent, nchanges: c_int, events: *mut Kevent, nevents: c_int, timeout: *const [i64; 2]) -> c_int {
    if nchanges < 0 || nevents < 0 {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let changes = unsafe { core::slice::from_raw_parts(changes.cast::<Kevent>(), if changes.is_null() { 0 } else { nchanges as usize }) };
    let room = if events.is_null() { 0 } else { nevents as usize };
    let mut out = 0usize;
    let push = |out: &mut usize, ev: Kevent| {
        if *out < room {
            // SAFETY: room for `room` events, the caller's guarantee.
            unsafe { events.add(*out).write(ev) };
            *out += 1;
        }
    };
    let deadline = if timeout.is_null() {
        None
    } else {
        // SAFETY: the caller's guarantee.
        let [s, n] = unsafe { *timeout };
        if s < 0 || !(0..NS as i64).contains(&n) {
            return fail(EINVAL) as c_int;
        }
        Some(now_ns() + s as u64 * NS + n as u64)
    };
    // The changes, under the lock.
    {
        let g = Guard::take();
        let t = g.table();
        let Some(q) = t.queues.iter_mut().find(|q| q.fd == kq) else {
            return fail(EBADF) as c_int;
        };
        let now = now_ns();
        for c in changes {
            match change(q, c, now) {
                Ok(()) if c.flags & EV_RECEIPT != 0 => push(&mut out, Kevent { flags: EV_ERROR | c.flags, data: 0, ..*c }),
                Ok(()) => {}
                Err(e) if room > 0 => push(&mut out, Kevent { flags: EV_ERROR | c.flags, data: i64::from(crate::errno::from_qnx(e)), ..*c }),
                Err(e) => return fail(e) as c_int,
            }
        }
    }
    if room == 0 || out > 0 && out == room {
        return out as c_int;
    }
    loop {
        // A snapshot of the queue's descriptors and timers.
        let mut fds = [Pollfd { fd: -1, events: 0, revents: 0 }; ENTRIES];
        let mut which = [usize::MAX; ENTRIES];
        let mut n = 0;
        let mut next_timer: Option<u64> = None;
        {
            let g = Guard::take();
            let t = g.table();
            let Some(q) = t.queues.iter_mut().find(|q| q.fd == kq) else {
                return fail(EBADF) as c_int;
            };
            let now = now_ns();
            for (i, e) in q.entries.iter_mut().enumerate() {
                if !e.used || !e.enabled {
                    continue;
                }
                match e.ev.filter {
                    EVFILT_READ | EVFILT_WRITE => {
                        fds[n] = Pollfd {
                            fd: e.ev.ident as c_int,
                            events: if e.ev.filter == EVFILT_READ { POLLIN } else { POLLOUT },
                            revents: 0,
                        };
                        which[n] = i;
                        n += 1;
                    }
                    EVFILT_TIMER => {
                        if e.deadline <= now {
                            let fired = if e.period > 0 { 1 + (now - e.deadline) / e.period } else { 1 };
                            push(&mut out, Kevent { data: fired as i64, ..e.ev });
                            if e.period > 0 && e.ev.flags & EV_ONESHOT == 0 {
                                e.deadline += fired * e.period;
                            } else {
                                *e = Entry::default();
                                continue;
                            }
                            if e.ev.flags & EV_DISPATCH != 0 {
                                e.enabled = false;
                            }
                        }
                        next_timer = Some(next_timer.map_or(e.deadline, |d| d.min(e.deadline)));
                    }
                    EVFILT_USER => {
                        if e.triggered {
                            push(&mut out, Kevent { data: 0, ..e.ev });
                            if e.ev.flags & EV_ONESHOT != 0 {
                                *e = Entry::default();
                                continue;
                            }
                            if e.ev.flags & EV_CLEAR != 0 {
                                e.triggered = false;
                            }
                            if e.ev.flags & EV_DISPATCH != 0 {
                                e.enabled = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let now = now_ns();
        let until = match (deadline, next_timer) {
            (Some(d), Some(t)) => Some(d.min(t)),
            (Some(d), None) => Some(d),
            (None, Some(t)) => Some(t),
            (None, None) => None,
        };
        // With events in hand already, only what is ready now is added.
        let wait_ns = if out > 0 { Some(0) } else { until.map(|u| u.saturating_sub(now)) };
        let spec = wait_ns.map(|w| [(w / NS) as i64, (w % NS) as i64]);
        // SAFETY: the descriptors are on this stack; the timeout too.
        let r = unsafe { __sys_ppoll(fds.as_mut_ptr(), n as u32, spec.as_ref().map_or(null(), |s| s), null()) };
        if r < 0 {
            return if out > 0 { out as c_int } else { -1 };
        }
        if r > 0 {
            let g = Guard::take();
            let t = g.table();
            if let Some(q) = t.queues.iter_mut().find(|q| q.fd == kq) {
                for i in 0..n {
                    let pf = fds[i];
                    if pf.revents == 0 {
                        continue;
                    }
                    let e = &mut q.entries[which[i]];
                    if !e.used {
                        continue;
                    }
                    let mut ev = e.ev;
                    if pf.revents & POLLNVAL != 0 {
                        ev.flags |= EV_ERROR;
                        ev.data = i64::from(crate::errno::from_qnx(EBADF));
                    } else {
                        if pf.revents & (POLLHUP | POLLERR) != 0 {
                            ev.flags |= EV_EOF;
                        }
                        ev.data = readable(pf.fd, e.ev.filter);
                    }
                    push(&mut out, ev);
                    if e.ev.flags & EV_ONESHOT != 0 {
                        *e = Entry::default();
                    } else if e.ev.flags & EV_DISPATCH != 0 {
                        e.enabled = false;
                    }
                }
            }
        }
        if out > 0 || deadline.is_some_and(|d| now_ns() >= d) {
            return out as c_int;
        }
    }
}

/// A read filter's data: the bytes waiting (`FIONREAD`), or 1 when the
/// server can't say; a write filter's: room for a write.
fn readable(fd: c_int, filter: i16) -> i64 {
    if filter != EVFILT_READ {
        return 65536;
    }
    const FIONREAD: u64 = 0x4004_667f;
    let mut n: c_int = 0;
    // SAFETY: an int, as the request says.
    if unsafe { crate::tty::__sys_ioctl(fd, FIONREAD, (&raw mut n).cast()) } == 0 && n > 0 {
        i64::from(n)
    } else {
        1
    }
}

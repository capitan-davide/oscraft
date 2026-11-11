//! The calls that are one message or one kernel call each: the time of
//! day, yielding, resource use, interval and POSIX timers, symbolic
//! links, owners, times, truncation, FIFOs, pathconf, flock, the
//! credentials, and the memory calls that are nothing on superlocked
//! memory.

use core::ffi::{c_char, c_int, c_void};
use core::ptr::{null, null_mut};

use proto::connect::{CONNECT_LINK, CONNECT_MKNOD, EXTRA_SYMLINK};
use proto::errno::{EINVAL, ENOENT, ENOSYS, ENOTSUP, EPERM};
use proto::event::{SIGEV_NONE as QNX_SIGEV_NONE, SIGEV_SIGNAL_CODE, Sigevent as Event};
use proto::fcntl::{self as q, SH_DENYNO};
use proto::io::{self as m, Chown, F_FLOCK, F_RDLCK, F_SETLK, F_SETLKW, F_UNLCK, F_WRLCK, Flock, Lock, LockReply, Pathconf, Utime, Utime64};
use proto::pod::Pod;
use proto::proc::{self as p, GetSetId};
use proto::stat::{S_IFIFO, S_IFLNK, S_IFMT};

use crate::errno::fail;
use crate::kercall::{self, Iov, SYSMGR_COID, msg_sendv, send};
use crate::path::{self, Request};
use crate::stat::Stat;

const BSD_AT_FDCWD: c_int = -100;
const BSD_AT_SYMLINK_NOFOLLOW: c_int = 0x200;
const BSD_O_WRONLY: c_int = 1;
const BSD_O_RDONLY: c_int = 0;
const QNX_CLOCK_REALTIME: c_int = 0;
const QNX_CLOCK_PROCESS_CPUTIME: c_int = 3;
const QNX_CLOCK_THREAD_CPUTIME: c_int = 4;
const NS: u64 = 1_000_000_000;

/// The directory a relative path starts from, or none for an absolute
/// path or the working directory.
///
/// # Safety
///
/// `path` is a string.
unsafe fn dir_of(dirfd: c_int, path: *const c_char) -> Result<Option<u16>, c_int> {
    // SAFETY: the caller's guarantee.
    if dirfd == BSD_AT_FDCWD || unsafe { path.read() } == b'/' as c_char {
        return Ok(None);
    }
    u16::try_from(dirfd).map(Some).map_err(|_| proto::errno::EBADF)
}

// ---- time ----

/// `gettimeofday(tv, tz)`: the realtime clock; no zone.
///
/// # Safety
///
/// `tv` is a timeval or null, `tz` a timezone or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_gettimeofday(tv: *mut [i64; 2], tz: *mut [c_int; 2]) -> c_int {
    if !tv.is_null() {
        let ns = match kercall::clock_time(QNX_CLOCK_REALTIME) {
            Ok(ns) => ns,
            Err(e) => return fail(e) as c_int,
        };
        // SAFETY: the caller's guarantee.
        unsafe { tv.write([(ns / NS) as i64, ((ns % NS) / 1000) as i64]) };
    }
    if !tz.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { tz.write([0, 0]) };
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_sched_yield() -> c_int {
    kercall::sched_yield().map_or_else(|e| fail(e) as c_int, |()| 0)
}

/// `getdtablesize()`: the descriptor limit, as `getrlimit()` tells.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_getdtablesize() -> c_int {
    const RLIMIT_NOFILE: c_int = 8;
    let mut lim = crate::process::Rlimit { cur: 0, max: 0 };
    // SAFETY: the limit is on this stack.
    if unsafe { crate::process::__sys_getrlimit(RLIMIT_NOFILE, &mut lim) } == 0 && (0..c_int::MAX as i64).contains(&lim.cur) {
        lim.cur as c_int
    } else {
        1024
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_vfork() -> c_int {
    crate::proc::__sys_fork()
}

/// `rfork(flags)`: a plain copy is `fork()`. Anything sharing more isn't
/// served: EINVAL, which sends libc's `posix_spawn()` to its `vfork()`
/// fallback for `RFSPAWN`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_rfork(flags: c_int) -> c_int {
    const RFFDG: c_int = 1 << 2;
    const RFPROC: c_int = 1 << 4;
    if flags == RFPROC | RFFDG {
        return crate::proc::__sys_fork();
    }
    fail(proto::errno::EINVAL) as c_int
}

/// `getrusage(who, ru)`: the CPU time of the process or the thread
/// (QNX's CPU-time clocks), all as user time; the rest zero.
///
/// # Safety
///
/// `ru` is a `struct rusage` (144 bytes).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getrusage(who: c_int, ru: *mut [i64; 18]) -> c_int {
    const RUSAGE_SELF: c_int = 0;
    const RUSAGE_CHILDREN: c_int = -1;
    const RUSAGE_THREAD: c_int = 1;
    let ns = match who {
        RUSAGE_SELF => kercall::clock_time(QNX_CLOCK_PROCESS_CPUTIME),
        RUSAGE_THREAD => kercall::clock_time(QNX_CLOCK_THREAD_CPUTIME),
        RUSAGE_CHILDREN => Ok(0),
        _ => return fail(EINVAL) as c_int,
    };
    let ns = match ns {
        Ok(ns) => ns,
        Err(e) => return fail(e) as c_int,
    };
    let mut r = [0i64; 18];
    r[0] = (ns / NS) as i64;
    r[1] = ((ns % NS) / 1000) as i64;
    // SAFETY: the caller's guarantee.
    unsafe { ru.write(r) };
    0
}

/// `struct itimerval` as `_itimer` (nsec, interval_nsec), and back.
fn itimer_of(v: &[i64; 4]) -> [u64; 2] {
    let ns = |s: i64, us: i64| (s.max(0) as u64) * NS + (us.max(0) as u64) * 1000;
    [ns(v[2], v[3]), ns(v[0], v[1])]
}

fn itimerval_of(it: &[u64; 2]) -> [i64; 4] {
    [(it[1] / NS) as i64, ((it[1] % NS) / 1000) as i64, (it[0] / NS) as i64, ((it[0] % NS) / 1000) as i64]
}

/// `setitimer(which, new, old)`: `ITIMER_REAL` is the process's alarm
/// (`TimerAlarm()`); the virtual and profiling timers have no QNX
/// counterpart (ENOTSUP).
///
/// # Safety
///
/// `new` is an itimerval, `old` one or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_setitimer(which: c_int, new: *const [i64; 4], old: *mut [i64; 4]) -> c_int {
    if which != 0 {
        return fail(ENOTSUP) as c_int;
    }
    if new.is_null() {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let it = itimer_of(unsafe { &*new });
    let mut was = [0u64; 2];
    // SAFETY: both are on this stack.
    match unsafe { kercall::timer_alarm(QNX_CLOCK_REALTIME, &it, &mut was) } {
        Ok(()) => {
            if !old.is_null() {
                // SAFETY: the caller's guarantee.
                unsafe { old.write(itimerval_of(&was)) };
            }
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `getitimer(which, cur)`.
///
/// # Safety
///
/// `cur` is an itimerval.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getitimer(which: c_int, cur: *mut [i64; 4]) -> c_int {
    if which != 0 {
        return fail(ENOTSUP) as c_int;
    }
    let mut was = [0u64; 2];
    // SAFETY: `was` is on this stack; a null `itime` only reads.
    match unsafe { kercall::timer_alarm(QNX_CLOCK_REALTIME, null(), &mut was) } {
        Ok(()) => {
            // SAFETY: the caller's guarantee.
            unsafe { cur.write(itimerval_of(&was)) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// FreeBSD's `struct sigevent` (`<sys/signal.h>`), as far as a timer
/// uses it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Sigevent {
    pub notify: c_int,
    pub signo: c_int,
    pub value: u64,
    pub un: [u64; 8],
}

const BSD_SIGEV_NONE: c_int = 0;
const BSD_SIGEV_SIGNAL: c_int = 1;
/// QNX's `SI_TIMER`, the code of a timer's signal.
const SI_TIMER: i16 = -3;

/// `ktimer_create(clock, event, id)`: `TimerCreate()`; a signal event
/// becomes `SIGEV_SIGNAL_CODE` with the value and `SI_TIMER`. A thread
/// event (`SIGEV_THREAD`, which libc serves itself, and
/// `SIGEV_THREAD_ID`) is not taken (EINVAL).
///
/// # Safety
///
/// `event` is a sigevent or null; `id` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_ktimer_create(clock: c_int, event: *const Sigevent, id: *mut c_int) -> c_int {
    let Some(qclock) = crate::time::qnx_clock(clock) else {
        return fail(EINVAL) as c_int;
    };
    let mut ev = Event::default();
    let evp = if event.is_null() {
        null()
    } else {
        // SAFETY: the caller's guarantee.
        let e = unsafe { *event };
        match e.notify {
            BSD_SIGEV_NONE => ev.notify = QNX_SIGEV_NONE,
            BSD_SIGEV_SIGNAL => {
                ev.notify = SIGEV_SIGNAL_CODE;
                ev.signo = e.signo;
                ev.value = e.value;
                ev.code = SI_TIMER;
            }
            _ => return fail(EINVAL) as c_int,
        }
        &raw const ev
    };
    // SAFETY: the event is on this stack, or null.
    match unsafe { kercall::timer_create(qclock, evp) } {
        Ok(t) => {
            // SAFETY: the caller's guarantee.
            unsafe { id.write(t) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_ktimer_delete(id: c_int) -> c_int {
    kercall::timer_destroy(id).map_or_else(|e| fail(e) as c_int, |()| 0)
}

/// `struct itimerspec` (interval, value) as `_itimer` (value, interval).
fn itimer_of_spec(v: &[i64; 4]) -> [u64; 2] {
    let ns = |s: i64, n: i64| (s.max(0) as u64) * NS + n.max(0) as u64;
    [ns(v[2], v[3]), ns(v[0], v[1])]
}

fn spec_of_itimer(it: &[u64; 2]) -> [i64; 4] {
    [(it[1] / NS) as i64, (it[1] % NS) as i64, (it[0] / NS) as i64, (it[0] % NS) as i64]
}

/// `ktimer_settime(id, flags, new, old)`: `TIMER_ABSTIME` is QNX's.
///
/// # Safety
///
/// `new` is an itimerspec, `old` one or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_ktimer_settime(id: c_int, flags: c_int, new: *const [i64; 4], old: *mut [i64; 4]) -> c_int {
    const BSD_TIMER_ABSTIME: c_int = 1;
    if new.is_null() {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let spec = unsafe { *new };
    if !(0..NS as i64).contains(&spec[1]) || !(0..NS as i64).contains(&spec[3]) {
        return fail(EINVAL) as c_int;
    }
    let it = itimer_of_spec(&spec);
    let qflags = if flags & BSD_TIMER_ABSTIME != 0 { kercall::TIMEOUT_ABSTIME } else { 0 };
    let mut was = [0u64; 2];
    // SAFETY: both are on this stack.
    match unsafe { kercall::timer_settime(id, qflags, &it, &mut was) } {
        Ok(()) => {
            if !old.is_null() {
                // SAFETY: the caller's guarantee.
                unsafe { old.write(spec_of_itimer(&was)) };
            }
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `ktimer_gettime(id, cur)`: the time left and the interval, from
/// `TimerInfo()`.
///
/// # Safety
///
/// `cur` is an itimerspec.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_ktimer_gettime(id: c_int, cur: *mut [i64; 4]) -> c_int {
    let mut info = [0u8; 88];
    // SAFETY: the info is on this stack.
    if let Err(e) = unsafe { kercall::timer_info(0, id, 0, &mut info) } {
        return fail(e) as c_int;
    }
    let at = |o: usize| u64::from_le_bytes(info[o..o + 8].try_into().unwrap_or([0; 8]));
    // `otime` at 16: the time left (0 when disarmed) and the interval.
    let left = if info[32] & 1 != 0 { at(16) } else { 0 };
    // SAFETY: the caller's guarantee.
    unsafe { cur.write(spec_of_itimer(&[left, at(24)])) };
    0
}

/// `ktimer_getoverrun(id)`: the overruns since the last ask
/// (`_NTO_TIMER_RESET_OVERRUNS`).
#[unsafe(no_mangle)]
pub extern "C" fn __sys_ktimer_getoverrun(id: c_int) -> c_int {
    const RESET_OVERRUNS: u32 = 0x2;
    let mut info = [0u8; 88];
    // SAFETY: the info is on this stack.
    match unsafe { kercall::timer_info(0, id, RESET_OVERRUNS, &mut info) } {
        Ok(_) => i32::from_le_bytes([info[48], info[49], info[50], info[51]]),
        Err(e) => fail(e) as c_int,
    }
}

// ---- paths ----

/// `symlinkat(target, dirfd, path)`: a connect request to make a link
/// node with the target as the extra data (`_IO_CONNECT_EXTRA_SYMLINK`).
///
/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_symlinkat(target: *const c_char, dirfd: c_int, path: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let mut len = 0;
    // SAFETY: a string: ends in a NUL.
    while unsafe { target.add(len).read() } != 0 {
        len += 1;
        if len >= path::PATH_MAX {
            return fail(proto::errno::ENAMETOOLONG) as c_int;
        }
    }
    // SAFETY: `len` bytes and the NUL.
    let extra = unsafe { core::slice::from_raw_parts(target.cast::<u8>(), len + 1) };
    let request = Request {
        subtype: CONNECT_LINK,
        file_type: 0,
        ioflag: q::O_CREAT | q::O_EXCL | q::O_NOCTTY,
        mode: S_IFLNK | 0o777,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: dir,
        extra_type: EXTRA_SYMLINK,
        extra,
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => {
            c.detach();
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_symlink(target: *const c_char, path: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_symlinkat(target, BSD_AT_FDCWD, path) }
}

fn chown_msg(uid: u32, gid: u32) -> Chown {
    Chown {
        r#type: m::IO_CHOWN,
        combine_len: size_of::<Chown>() as u16,
        gid: gid as i32,
        uid: uid as i32,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_fchown(fd: c_int, uid: u32, gid: u32) -> c_int {
    send(fd, &chown_msg(uid, gid)).map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `fchownat(dirfd, path, uid, gid, flag)`: a probe combined with
/// `_IO_CHOWN`, as `fchmodat()`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_fchownat(dirfd: c_int, path: *const c_char, uid: u32, gid: u32, flag: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let msg = chown_msg(uid, gid);
    let link = if flag & BSD_AT_SYMLINK_NOFOLLOW != 0 { S_IFLNK } else { 0 };
    // SAFETY: the caller's guarantee.
    match unsafe { path::probe(dir, link, msg.as_bytes(), path, &mut []) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_chown(path: *const c_char, uid: u32, gid: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_fchownat(BSD_AT_FDCWD, path, uid, gid, 0) }
}

/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_lchown(path: *const c_char, uid: u32, gid: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_fchownat(BSD_AT_FDCWD, path, uid, gid, BSD_AT_SYMLINK_NOFOLLOW) }
}

/// FreeBSD's `UTIME_NOW` and `UTIME_OMIT` in a timespec's nanoseconds.
const UTIME_NOW: i64 = -1;
const UTIME_OMIT: i64 = -2;

/// The `_IO_UTIME64` for the times `t` (two timespecs, or null for
/// now), with the file's own times (`st`) where one is omitted: `None`
/// for both omitted, or the 32-bit `_IO_UTIME` form for both now.
enum Times {
    Now,
    Keep,
    Set(Utime64),
}

/// # Safety
///
/// `t` is two timespecs or null.
unsafe fn times_of(t: *const [i64; 4], st: impl FnOnce() -> Result<Stat, c_int>) -> Result<Times, c_int> {
    if t.is_null() {
        return Ok(Times::Now);
    }
    // SAFETY: the caller's guarantee.
    let [mut a_s, a_n, mut m_s, m_n] = unsafe { *t };
    if a_n == UTIME_NOW && m_n == UTIME_NOW {
        return Ok(Times::Now);
    }
    if a_n == UTIME_OMIT && m_n == UTIME_OMIT {
        return Ok(Times::Keep);
    }
    let (mut a_n, mut m_n) = (a_n, m_n);
    if a_n == UTIME_NOW || m_n == UTIME_NOW || a_n == UTIME_OMIT || m_n == UTIME_OMIT {
        let now = kercall::clock_time(QNX_CLOCK_REALTIME)?;
        let (now_s, now_n) = ((now / NS) as i64, (now % NS) as i64);
        let old = if a_n == UTIME_OMIT || m_n == UTIME_OMIT { Some(st()?) } else { None };
        let fix = |s: &mut i64, n: &mut i64, old_t: [i64; 2]| {
            if *n == UTIME_NOW {
                (*s, *n) = (now_s, now_n);
            } else if *n == UTIME_OMIT {
                (*s, *n) = (old_t[0], old_t[1]);
            }
        };
        fix(&mut a_s, &mut a_n, old.map_or([0, 0], |o| o.st_atim));
        fix(&mut m_s, &mut m_n, old.map_or([0, 0], |o| o.st_mtim));
    }
    if !(0..NS as i64).contains(&a_n) || !(0..NS as i64).contains(&m_n) {
        return Err(EINVAL);
    }
    Ok(Times::Set(Utime64 {
        r#type: m::IO_UTIME64,
        combine_len: size_of::<Utime64>() as u16,
        zero: 0,
        atime_s: a_s,
        atime_ns: a_n,
        mtime_s: m_s,
        mtime_ns: m_n,
    }))
}

fn utime_now() -> Utime {
    Utime {
        r#type: m::IO_UTIME,
        combine_len: size_of::<Utime>() as u16,
        cur_flag: 1,
        actime: 0,
        modtime: 0,
    }
}

/// `futimens(fd, times)`.
///
/// # Safety
///
/// `times` is two timespecs or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_futimens(fd: c_int, times: *const [i64; 4]) -> c_int {
    let st = || {
        let mut sb = Stat::default();
        // SAFETY: the stat is on this stack.
        if unsafe { crate::stat::__sys_fstat(fd, &mut sb) } != 0 { Err(EINVAL) } else { Ok(sb) }
    };
    // SAFETY: the caller's guarantee.
    let r = match unsafe { times_of(times, st) } {
        Ok(Times::Now) => send(fd, &utime_now()),
        Ok(Times::Keep) => Ok(0),
        Ok(Times::Set(u)) => send(fd, &u),
        Err(e) => Err(e),
    };
    r.map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `utimensat(dirfd, path, times, flag)`: a probe combined with the
/// utime message.
///
/// # Safety
///
/// `path` is a string; `times` two timespecs or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_utimensat(dirfd: c_int, path: *const c_char, times: *const [i64; 4], flag: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let link = if flag & BSD_AT_SYMLINK_NOFOLLOW != 0 { S_IFLNK } else { 0 };
    let st = || {
        let mut sb = Stat::default();
        // SAFETY: the caller's guarantee; the stat is on this stack.
        if unsafe { crate::fs::__sys_fstatat(dirfd, path, &mut sb, flag & BSD_AT_SYMLINK_NOFOLLOW) } != 0 { Err(EINVAL) } else { Ok(sb) }
    };
    // SAFETY: the caller's guarantee.
    let (now, set);
    let bytes: &[u8] = match unsafe { times_of(times, st) } {
        Ok(Times::Keep) => return 0,
        Err(e) => return fail(e) as c_int,
        Ok(Times::Now) => {
            now = utime_now();
            now.as_bytes()
        }
        Ok(Times::Set(u)) => {
            set = u;
            set.as_bytes()
        }
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::probe(dir, link, bytes, path, &mut []) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// Two timevals as two timespecs.
///
/// # Safety
///
/// `tv` is two timevals or null.
unsafe fn specs_of(tv: *const [i64; 4]) -> Option<[i64; 4]> {
    if tv.is_null() {
        return None;
    }
    // SAFETY: the caller's guarantee.
    let v = unsafe { *tv };
    Some([v[0], v[1] * 1000, v[2], v[3] * 1000])
}

/// # Safety
///
/// `path` is a string; `tv` two timevals or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_utimes(path: *const c_char, tv: *const [i64; 4]) -> c_int {
    // SAFETY: the caller's guarantee.
    let ts = unsafe { specs_of(tv) };
    // SAFETY: as above.
    unsafe { __sys_utimensat(BSD_AT_FDCWD, path, ts.as_ref().map_or(null(), |t| t), 0) }
}

/// # Safety
///
/// As `utimes()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_lutimes(path: *const c_char, tv: *const [i64; 4]) -> c_int {
    // SAFETY: the caller's guarantee.
    let ts = unsafe { specs_of(tv) };
    // SAFETY: as above.
    unsafe { __sys_utimensat(BSD_AT_FDCWD, path, ts.as_ref().map_or(null(), |t| t), BSD_AT_SYMLINK_NOFOLLOW) }
}

/// # Safety
///
/// As `utimes()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_futimesat(dirfd: c_int, path: *const c_char, tv: *const [i64; 4]) -> c_int {
    // SAFETY: the caller's guarantee.
    let ts = unsafe { specs_of(tv) };
    // SAFETY: as above.
    unsafe { __sys_utimensat(dirfd, path, ts.as_ref().map_or(null(), |t| t), 0) }
}

/// # Safety
///
/// `tv` is two timevals or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_futimes(fd: c_int, tv: *const [i64; 4]) -> c_int {
    // SAFETY: the caller's guarantee.
    let ts = unsafe { specs_of(tv) };
    // SAFETY: as above.
    unsafe { __sys_futimens(fd, ts.as_ref().map_or(null(), |t| t)) }
}

/// `truncate(path, length)`: an open for writing and `ftruncate()`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_truncate(path: *const c_char, length: i64) -> c_int {
    // SAFETY: the caller's guarantee.
    let fd = unsafe { crate::fs::__sys_open(path, BSD_O_WRONLY, 0) };
    if fd < 0 {
        return -1;
    }
    let r = crate::fd::__sys_ftruncate(fd, length);
    let e = crate::errno::last();
    let _ = crate::fs::__sys_close(fd);
    crate::errno::set_last(e);
    r
}

/// `mkfifoat(dirfd, path, mode)`: a node of `S_IFIFO`, which the path
/// manager gives the pipe manager.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mkfifoat(dirfd: c_int, path: *const c_char, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let request = Request {
        subtype: CONNECT_MKNOD,
        file_type: 0,
        ioflag: q::O_NOCTTY | q::O_CREAT,
        mode: (mode & !S_IFMT) | S_IFIFO,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: dir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => {
            c.detach();
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mkfifo(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_mkfifoat(BSD_AT_FDCWD, path, mode) }
}

/// QNX's `_PC_*` number for FreeBSD's (`<sys/unistd.h>` and QNX's
/// `<confname.h>`: the first six agree, the next three are in another
/// order, the I/O ones far apart).
fn pc_name(name: c_int) -> Option<i16> {
    Some(match name {
        1..=6 => name as i16,
        7 => 9,  // _PC_CHOWN_RESTRICTED
        8 => 7,  // _PC_NO_TRUNC
        9 => 8,  // _PC_VDISABLE
        53 => 12, // _PC_ASYNC_IO
        54 => 13, // _PC_PRIO_IO
        55 => 14, // _PC_SYNC_IO
        _ => return None,
    })
}

/// `fpathconf(fd, name)`: `_IO_PATHCONF`; the reply's status is the
/// value.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_fpathconf(fd: c_int, name: c_int) -> isize {
    let Some(qname) = pc_name(name) else {
        return fail(EINVAL);
    };
    let msg = Pathconf {
        r#type: m::IO_PATHCONF,
        combine_len: size_of::<Pathconf>() as u16,
        name: qname,
        zero: 0,
    };
    match send(fd, &msg) {
        Ok(v) => v as isize,
        Err(e) => fail(e),
    }
}

/// `pathconf(path, name)`: on an open of the path.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_pathconf(path: *const c_char, name: c_int) -> isize {
    // SAFETY: the caller's guarantee.
    let fd = unsafe { crate::fs::__sys_open(path, BSD_O_RDONLY, 0) };
    if fd < 0 {
        return -1;
    }
    let r = __sys_fpathconf(fd, name);
    let e = crate::errno::last();
    let _ = crate::fs::__sys_close(fd);
    crate::errno::set_last(e);
    r
}

/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_lpathconf(path: *const c_char, name: c_int) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_pathconf(path, name) }
}

/// `flock(fd, op)`: an `_IO_LOCK` of the open file (`F_FLOCK` in the
/// type), waiting unless `LOCK_NB`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_flock(fd: c_int, op: c_int) -> c_int {
    const LOCK_SH: c_int = 1;
    const LOCK_EX: c_int = 2;
    const LOCK_NB: c_int = 4;
    const LOCK_UN: c_int = 8;
    let kind = match op & !LOCK_NB {
        LOCK_SH => F_RDLCK,
        LOCK_EX => F_WRLCK,
        LOCK_UN => F_UNLCK,
        _ => return fail(EINVAL) as c_int,
    };
    let msg = Lock {
        r#type: m::IO_LOCK,
        combine_len: (size_of::<Lock>() + size_of::<Flock>()) as u16,
        subtype: if op & LOCK_NB != 0 || kind == F_UNLCK { F_SETLK } else { F_SETLKW },
        nbytes: size_of::<Flock>() as u32,
    };
    let lock = Flock {
        l_type: kind | F_FLOCK,
        ..Flock::default()
    };
    let mut reply = LockReply::default();
    let send_parts = [Iov::of(&msg), Iov::of(&lock)];
    let recv = [Iov::raw((&raw mut reply).cast(), size_of::<LockReply>())];
    // SAFETY: all are on this stack.
    match unsafe { msg_sendv(fd, &send_parts, &recv) } {
        Ok(_) => 0,
        Err(proto::errno::EAGAIN) if op & LOCK_NB != 0 => fail(proto::errno::EWOULDBLOCK) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

// ---- credentials ----

/// A `_PROC_GETSETID` of subtype `subtype` with `eid` and `rid`
/// (`u32::MAX`: unchanged), and the groups.
fn set_id(subtype: u16, eid: u32, rid: u32, groups: &[u32]) -> c_int {
    let request = GetSetId {
        r#type: p::PROC_GETSETID,
        subtype,
        pid: 0,
        eid,
        rid,
        ngroups: groups.len() as i32,
    };
    let mut reply = [0u8; p::GETSETID_REPLY_SIZE];
    let send_parts = [Iov::of(&request), Iov::raw(groups.as_ptr().cast(), size_of_val(groups))];
    let recv = [Iov::raw(reply.as_mut_ptr().cast(), reply.len())];
    // SAFETY: all are on this stack or the caller's slice.
    match unsafe { msg_sendv(SYSMGR_COID, &send_parts, &recv) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

const NONE: u32 = u32::MAX;

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setuid(uid: u32) -> c_int {
    set_id(p::ID_SETUID, uid, NONE, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_seteuid(uid: u32) -> c_int {
    set_id(p::ID_SETEUID, uid, NONE, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setreuid(ruid: u32, euid: u32) -> c_int {
    set_id(p::ID_SETREUID, euid, ruid, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setgid(gid: u32) -> c_int {
    set_id(p::ID_SETGID, gid, NONE, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setegid(gid: u32) -> c_int {
    set_id(p::ID_SETEGID, gid, NONE, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setregid(rgid: u32, egid: u32) -> c_int {
    set_id(p::ID_SETREGID, egid, rgid, &[])
}

/// `setresuid(r, e, s)`: QNX sets the saved ID along with the real one
/// (`setreuid()`); a saved ID of its own is EINVAL.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_setresuid(ruid: u32, euid: u32, suid: u32) -> c_int {
    if suid != NONE && suid != ruid && suid != euid {
        return fail(EINVAL) as c_int;
    }
    set_id(p::ID_SETREUID, euid, ruid, &[])
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setresgid(rgid: u32, egid: u32, sgid: u32) -> c_int {
    if sgid != NONE && sgid != rgid && sgid != egid {
        return fail(EINVAL) as c_int;
    }
    set_id(p::ID_SETREGID, egid, rgid, &[])
}

/// # Safety
///
/// The three are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getresuid(r: *mut u32, e: *mut u32, s: *mut u32) -> c_int {
    match crate::cred::ids() {
        Ok(ids) => {
            // SAFETY: the caller's guarantee.
            unsafe {
                r.write(ids.ruid);
                e.write(ids.euid);
                s.write(ids.suid);
            }
            0
        }
        Err(err) => fail(err) as c_int,
    }
}

/// # Safety
///
/// The three are writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getresgid(r: *mut u32, e: *mut u32, s: *mut u32) -> c_int {
    match crate::cred::ids() {
        Ok(ids) => {
            // SAFETY: the caller's guarantee.
            unsafe {
                r.write(ids.rgid);
                e.write(ids.egid);
                s.write(ids.sgid);
            }
            0
        }
        Err(err) => fail(err) as c_int,
    }
}

/// `getgroups(n, list)`: how many there are; with room, copied.
///
/// # Safety
///
/// `list` holds `n` ids.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getgroups(n: c_int, list: *mut u32) -> c_int {
    let ids = match crate::cred::ids() {
        Ok(ids) => ids,
        Err(e) => return fail(e) as c_int,
    };
    let count = (ids.ngroups as usize).min(ids.grouplist.len());
    if n == 0 {
        return count as c_int;
    }
    if (n as usize) < count {
        return fail(EINVAL) as c_int;
    }
    for (i, g) in ids.grouplist[..count].iter().enumerate() {
        // SAFETY: the caller's guarantee: room for `count`.
        unsafe { list.add(i).write(*g) };
    }
    count as c_int
}

/// `setgroups(n, list)`.
///
/// # Safety
///
/// `list` holds `n` ids.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_setgroups(n: c_int, list: *const u32) -> c_int {
    if !(0..=8).contains(&n) {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let groups = unsafe { core::slice::from_raw_parts(list, n as usize) };
    set_id(p::ID_SETGROUPS, NONE, NONE, groups)
}

// ---- memory: superlocked, so already locked, synced and resident ----

#[unsafe(no_mangle)]
pub extern "C" fn __sys_mlock(_addr: *const c_void, _len: usize) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_munlock(_addr: *const c_void, _len: usize) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_mlockall(_flags: c_int) -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_munlockall() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_msync(_addr: *const c_void, _len: usize, _flags: c_int) -> c_int {
    0
}

/// `mincore(addr, len, vec)`: every page resident.
///
/// # Safety
///
/// `vec` holds a byte per page of the range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mincore(addr: *const c_void, len: usize, vec: *mut u8) -> c_int {
    const MINCORE_INCORE: u8 = 1;
    let page = 4096usize;
    let start = addr as usize & !(page - 1);
    let pages = (addr as usize + len).div_ceil(page).saturating_sub(start / page);
    for i in 0..pages {
        // SAFETY: the caller's guarantee.
        unsafe { vec.add(i).write(MINCORE_INCORE) };
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_posix_fadvise(_fd: c_int, _offset: i64, _len: i64, _advice: c_int) -> c_int {
    0
}

/// `sync()`: nothing to flush that a server does not flush itself.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sync() {
    let _ = (ENOSYS, EPERM, null_mut::<u8>());
}

// ---- Capsicum: no sandbox here; the helpers take ENOSYS as "none" ----

/// `cap_enter()` and the rights limits: ENOSYS, quietly (FreeBSD's
/// programs ask and go on without a sandbox when the kernel has none).
#[unsafe(no_mangle)]
pub extern "C" fn __sys_cap_enter() -> c_int {
    fail(ENOSYS) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_cap_rights_limit(_fd: c_int, _rights: *const c_void) -> c_int {
    fail(ENOSYS) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_cap_ioctls_limit(_fd: c_int, _cmds: *const c_void, _n: usize) -> c_int {
    fail(ENOSYS) as c_int
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_cap_fcntls_limit(_fd: c_int, _rights: u32) -> c_int {
    fail(ENOSYS) as c_int
}

/// `cap_getmode(mode)`: not in capability mode.
///
/// # Safety
///
/// `mode` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_cap_getmode(mode: *mut u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { mode.write(0) };
    0
}

/// `copy_file_range(infd, inoff, outfd, outoff, len, flags)`: reads and
/// writes, through a buffer, at the offsets given (which advance) or
/// the descriptors' own; how many bytes went.
///
/// # Safety
///
/// `inoff` and `outoff` are null or writable offsets.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_copy_file_range(infd: c_int, inoff: *mut i64, outfd: c_int, outoff: *mut i64, len: usize, flags: u32) -> isize {
    if flags != 0 {
        return fail(EINVAL);
    }
    let mut buf = [0u8; 16384];
    let mut done = 0usize;
    while done < len {
        let want = (len - done).min(buf.len());
        // SAFETY: the buffer is on this stack; the offsets the caller's.
        let n = unsafe {
            if inoff.is_null() {
                crate::io::__sys_read(infd, buf.as_mut_ptr().cast(), want)
            } else {
                crate::fd::__sys_pread(infd, buf.as_mut_ptr().cast(), want, *inoff)
            }
        };
        if n < 0 {
            return if done > 0 { done as isize } else { -1 };
        }
        if n == 0 {
            break;
        }
        let mut written = 0usize;
        while written < n as usize {
            // SAFETY: as above.
            let w = unsafe {
                if outoff.is_null() {
                    crate::io::__sys_write(outfd, buf.as_ptr().add(written).cast(), n as usize - written)
                } else {
                    crate::fd::__sys_pwrite(outfd, buf.as_ptr().add(written).cast(), n as usize - written, *outoff)
                }
            };
            if w <= 0 {
                return if done + written > 0 { (done + written) as isize } else { -1 };
            }
            if !outoff.is_null() {
                // SAFETY: the caller's guarantee.
                unsafe { *outoff += w as i64 };
            }
            written += w as usize;
        }
        if !inoff.is_null() {
            // SAFETY: the caller's guarantee.
            unsafe { *inoff += n as i64 };
        }
        done += n as usize;
    }
    done as isize
}

// Kernel modules: netd's (`/dev/netd`), which loads them into its stack
// with FreeBSD's linker. The call goes as it is: its FreeBSD number, two
// integers and a buffer (netd's `io_kld`, bsdkern's `kern/kld.c`).

/// `/dev/netd`'s command, to and from, its 4096 bytes.
const DCMD_KLD: u32 = 0xc000_0000 | (4096 << 16) | 0x4e01;
const KLD_ROOM: usize = 4080;

/// One modules' call: its return, the buffer back in `buf`. `None` with no
/// netd to ask.
fn kld(number: c_int, a0: c_int, a1: c_int, buf: &mut [u8; KLD_ROOM]) -> Option<Result<i64, c_int>> {
    // SAFETY: a string.
    let fd = unsafe { crate::fs::__sys_open(c"/dev/netd".as_ptr(), q::O_RDWR as c_int | q::O_CLOEXEC as c_int, 0) };
    if fd < 0 {
        return None;
    }
    let mut msg = [0u8; 16 + KLD_ROOM];
    msg[0..4].copy_from_slice(&number.to_le_bytes());
    msg[4..8].copy_from_slice(&a0.to_le_bytes());
    msg[8..12].copy_from_slice(&a1.to_le_bytes());
    msg[16..].copy_from_slice(buf);
    // SAFETY: the buffer on this stack, as the command's size says.
    let r = unsafe { crate::tty::devctl(fd, DCMD_KLD, msg.as_mut_ptr().cast(), msg.len()) };
    let _ = crate::fs::__sys_close(fd);
    Some(r.map(|_| {
        buf.copy_from_slice(&msg[16..]);
        i64::from_le_bytes(msg[0..8].try_into().unwrap())
    }))
}

fn kld_result(r: Option<Result<i64, c_int>>, absent: isize) -> isize {
    match r {
        Some(Ok(v)) => v as isize,
        Some(Err(e)) => fail(e),
        None => absent,
    }
}

/// A string argument in the buffer.
///
/// # Safety
///
/// `s` is a string.
unsafe fn kld_str(s: *const c_char) -> Result<[u8; KLD_ROOM], c_int> {
    let mut buf = [0u8; KLD_ROOM];
    for (i, slot) in buf.iter_mut().enumerate().take(KLD_ROOM - 1) {
        // SAFETY: the caller's guarantee, up to its NUL.
        let c = unsafe { s.add(i).read() } as u8;
        *slot = c;
        if c == 0 {
            return Ok(buf);
        }
    }
    Err(proto::errno::ENAMETOOLONG)
}

/// # Safety
///
/// `file` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_kldload(file: *const c_char) -> isize {
    // SAFETY: the caller's guarantee.
    match unsafe { kld_str(file) } {
        Ok(mut b) => kld_result(kld(304, 0, 0, &mut b), fail(ENOENT)),
        Err(e) => fail(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_kldunload(fileid: c_int) -> isize {
    kld_result(kld(305, fileid, 0, &mut [0; KLD_ROOM]), fail(ENOENT))
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_kldunloadf(fileid: c_int, flags: c_int) -> isize {
    kld_result(kld(444, fileid, flags, &mut [0; KLD_ROOM]), fail(ENOENT))
}

/// # Safety
///
/// `file` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_kldfind(file: *const c_char) -> isize {
    // SAFETY: the caller's guarantee.
    match unsafe { kld_str(file) } {
        Ok(mut b) => kld_result(kld(306, 0, 0, &mut b), fail(ENOENT)),
        Err(e) => fail(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_kldnext(fileid: c_int) -> isize {
    kld_result(kld(307, fileid, 0, &mut [0; KLD_ROOM]), 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_kldfirstmod(fileid: c_int) -> isize {
    kld_result(kld(309, fileid, 0, &mut [0; KLD_ROOM]), fail(ENOENT))
}

/// A stat call: the structure's `version` (its size, its first int) says
/// how much goes each way.
///
/// # Safety
///
/// `stat` is writable for the size its version says.
unsafe fn kld_stat(number: c_int, id: c_int, stat: *mut u8) -> isize {
    if stat.is_null() {
        return fail(proto::errno::EFAULT);
    }
    // SAFETY: the caller's guarantee: the version comes first.
    let size = (unsafe { stat.cast::<c_int>().read_unaligned() } as usize).min(KLD_ROOM);
    let mut b = [0u8; KLD_ROOM];
    // SAFETY: as above.
    unsafe { core::ptr::copy_nonoverlapping(stat, b.as_mut_ptr(), size) };
    let r = kld(number, id, 0, &mut b);
    if let Some(Ok(_)) = r {
        // SAFETY: as above.
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), stat, size) };
    }
    kld_result(r, fail(ENOENT))
}

/// # Safety
///
/// As `kld_stat()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_kldstat(fileid: c_int, stat: *mut c_void) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { kld_stat(308, fileid, stat.cast()) }
}

/// # Safety
///
/// `name` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_modfind(name: *const c_char) -> isize {
    // SAFETY: the caller's guarantee.
    match unsafe { kld_str(name) } {
        // Without netd, what a program asks for is built in, or not there.
        Ok(mut b) => kld_result(kld(303, 0, 0, &mut b), 1),
        Err(e) => fail(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_modnext(modid: c_int) -> isize {
    kld_result(kld(300, modid, 0, &mut [0; KLD_ROOM]), 0)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_modfnext(modid: c_int) -> isize {
    kld_result(kld(302, modid, 0, &mut [0; KLD_ROOM]), 0)
}

/// # Safety
///
/// As `kld_stat()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_modstat(modid: c_int, stat: *mut c_void) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { kld_stat(301, modid, stat.cast()) }
}

/// `sysarch(op, parms)`. `ARM64_GUARD_PAGE` (rtld's, for an object built
/// with BTI): accepted, with nothing to do, as FreeBSD on a CPU without
/// BTI; OsCraft's page tables don't guard branch targets. Anything else is
/// EINVAL, as FreeBSD's for an operation it doesn't know.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_sysarch(op: c_int, _parms: *mut c_void) -> isize {
    const ARM64_GUARD_PAGE: c_int = 0x100;
    if op == ARM64_GUARD_PAGE { 0 } else { fail(EINVAL) }
}

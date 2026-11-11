//! `_umtx_op()`: FreeBSD's kernel-assisted locks (`<sys/umtx.h>`), on
//! which libthr builds mutexes, condition variables, semaphores and
//! read-write locks, with the fast paths in user space and the waits in
//! the kernel. OsCraft's kernel gives a futex (`kercall::futex_wait`,
//! `futex_wake`); each operation here is FreeBSD's `kern_umtx.c`
//! semantics on top of it, the way a libc builds the same on Linux:
//!
//! - a mutex's unlock wakes one waiter and, while others still wait (the
//!   futex says how many), keeps the mutex marked contested so that the
//!   next unlock wakes the next, as FreeBSD's `do_wake2_umutex` does from
//!   its own count; the mark is cleared when none wait;
//! - a condition variable's `c_spare[0]` is a sequence number: a signal
//!   steps it, a waiter waits while it is unchanged, so a signal between
//!   the unlock and the wait is not lost;
//! - a mutex that inherits priority waits with the holder's thread ID,
//!   which the kernel raises to the waiter's priority meanwhile
//!   (`FUTEX_WAIT_PI`); a priority ceiling is not kept;
//! - a read-write lock's waiters, readers and writers alike, wait on its
//!   state word, and an unlock wakes them all to try again, where FreeBSD
//!   keeps a queue for each and wakes what fits.
//!
//! Timeouts arm `TimerTimeout()` for the wait (`_NTO_TIMEOUT_SEM`).

use core::ffi::{c_int, c_void};
use core::sync::atomic::{AtomicU32, Ordering};

use proto::errno::{EAGAIN, EINTR, EINVAL, ENOSYS, EPERM, ETIMEDOUT};

use crate::errno::{fail, not_implemented};
use crate::kercall::{TIMEOUT_ABSTIME, TIMEOUT_SEM, futex_wait, futex_wait_pi, futex_wake, thr_self, timer_timeout};
use crate::time::qnx_clock;

const OP_WAIT: c_int = 2;
const OP_WAKE: c_int = 3;
const OP_MUTEX_TRYLOCK: c_int = 4;
const OP_MUTEX_LOCK: c_int = 5;
const OP_MUTEX_UNLOCK: c_int = 6;
const OP_SET_CEILING: c_int = 7;
const OP_CV_WAIT: c_int = 8;
const OP_CV_SIGNAL: c_int = 9;
const OP_CV_BROADCAST: c_int = 10;
const OP_WAIT_UINT: c_int = 11;
const OP_WAIT_UINT_PRIVATE: c_int = 15;
const OP_WAKE_PRIVATE: c_int = 16;
const OP_MUTEX_WAIT: c_int = 17;
const OP_MUTEX_WAKE: c_int = 18;
const OP_NWAKE_PRIVATE: c_int = 21;
const OP_MUTEX_WAKE2: c_int = 22;
const OP_SEM2_WAIT: c_int = 23;
const OP_SEM2_WAKE: c_int = 24;
const OP_RW_RDLOCK: c_int = 12;
const OP_RW_WRLOCK: c_int = 13;
const OP_RW_UNLOCK: c_int = 14;
const OP_SHM: c_int = 25;
const OP_ROBUST_LISTS: c_int = 26;
const OP_GET_MIN_TIMEOUT: c_int = 27;
const OP_SET_MIN_TIMEOUT: c_int = 28;

const UMUTEX_CONTESTED: u32 = 0x8000_0000;
const UMUTEX_RB_OWNERDEAD: u32 = UMUTEX_CONTESTED | 0x10;
const UMUTEX_RB_NOTRECOV: u32 = UMUTEX_CONTESTED | 0x11;
const UMUTEX_UNOWNED: u32 = 0;
const UMUTEX_PRIO_INHERIT: u32 = 0x4;
const USEM_HAS_WAITERS: u32 = 0x8000_0000;
const URWLOCK_PREFER_READER: u32 = 0x2;
const URWLOCK_WRITE_OWNER: u32 = 0x8000_0000;
const URWLOCK_WRITE_WAITERS: u32 = 0x4000_0000;
const URWLOCK_READ_WAITERS: u32 = 0x2000_0000;
const URWLOCK_MAX_READERS: u32 = 0x1fff_ffff;
const CVWAIT_ABSTIME: u64 = 0x02;
const CVWAIT_CLOCKID: u64 = 0x04;
const CVWAIT_UMTX_TIME: u64 = 0x08;
const UMTX_ABSTIME: u32 = 0x01;
const BSD_CLOCK_REALTIME: c_int = 0;

/// `struct umutex`: the owner word first.
#[repr(C)]
struct Umutex {
    m_owner: AtomicU32,
    m_flags: u32,
}

/// `struct ucond`.
#[repr(C)]
struct Ucond {
    c_has_waiters: AtomicU32,
    c_flags: u32,
    c_clockid: u32,
    /// The sequence number (`c_spare[0]`).
    c_seq: AtomicU32,
}

/// `struct urwlock`: the state (the writer bit, the waiters' bits, the
/// readers' count), the flags, the counts of blocked readers and writers.
#[repr(C)]
struct Urwlock {
    state: AtomicU32,
    flags: u32,
    blocked_readers: AtomicU32,
    blocked_writers: AtomicU32,
}

/// A wait's deadline: a clock and an absolute time on it, in nanoseconds.
struct Deadline {
    clock: c_int,
    ns: u64,
}

impl Deadline {
    /// Arms the timeout for the wait that follows.
    fn arm(&self) -> Result<(), c_int> {
        let clock = qnx_clock(self.clock).ok_or(EINVAL)?;
        timer_timeout(clock, TIMEOUT_SEM | TIMEOUT_ABSTIME, self.ns)
    }
}

/// The deadline from `_umtx_op`'s timeout arguments: with `size` 0,
/// `timeout` is a relative `struct timespec` (or null for none); with
/// `size` of `struct _umtx_time`, that, absolute if it says so.
///
/// # Safety
///
/// `timeout` is as `size` says, or null.
unsafe fn deadline(size: usize, timeout: *const c_void) -> Result<Option<Deadline>, c_int> {
    if timeout.is_null() {
        return Ok(None);
    }
    // SAFETY: the caller's guarantee: at least a timespec.
    let [sec, nsec] = unsafe { timeout.cast::<[i64; 2]>().read_unaligned() };
    let (absolute, clock) = if size >= 24 {
        // SAFETY: a `_umtx_time`: the timespec, then flags and clock.
        let [flags, clockid] = unsafe { timeout.cast::<u8>().add(16).cast::<[u32; 2]>().read_unaligned() };
        (flags & UMTX_ABSTIME != 0, clockid as c_int)
    } else {
        (false, BSD_CLOCK_REALTIME)
    };
    if !(0..1_000_000_000).contains(&nsec) || sec < 0 {
        return Err(EINVAL);
    }
    let given = sec as u64 * 1_000_000_000 + nsec as u64;
    let ns = if absolute {
        given
    } else {
        let now = crate::kercall::clock_time(qnx_clock(clock).ok_or(EINVAL)?)?;
        now + given
    };
    Ok(Some(Deadline { clock, ns }))
}

/// Waits while `*word == val`, until `deadline`: 0 when woken or when
/// the word differs, else ETIMEDOUT or EINTR.
///
/// # Safety
///
/// `word` is a word of the process.
unsafe fn wait(word: *const u32, val: u32, deadline: &Option<Deadline>) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { wait_owned(word, val, deadline, None) }
}

/// [`wait`], raising the thread `owner` names meanwhile, for a mutex
/// that inherits priority.
///
/// # Safety
///
/// As [`wait`].
unsafe fn wait_owned(word: *const u32, val: u32, deadline: &Option<Deadline>, owner: Option<u32>) -> c_int {
    if let Some(d) = deadline
        && let Err(e) = d.arm()
    {
        return e;
    }
    // SAFETY: the caller's guarantee.
    let r = match owner {
        Some(o) => unsafe { futex_wait_pi(word, val, o) },
        None => unsafe { futex_wait(word, val) },
    };
    match r {
        Ok(()) | Err(EAGAIN) => 0,
        Err(e) => e,
    }
}

/// The holder to raise while waiting for `m`: its thread ID, for a
/// mutex that inherits priority.
fn pi_owner(m: &Umutex, owner: u32) -> Option<u32> {
    (m.m_flags & UMUTEX_PRIO_INHERIT != 0).then_some(crate::kercall::kernel_tid(i64::from(owner & !UMUTEX_CONTESTED)) as u32)
}

/// `_umtx_op(obj, op, val, uaddr, uaddr2)`: the operation's result, as
/// `errno` with -1.
///
/// # Safety
///
/// `obj`, `uaddr` and `uaddr2` are what `op` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys__umtx_op(obj: *mut c_void, op: c_int, val: u64, uaddr: *mut c_void, uaddr2: *mut c_void) -> c_int {
    // SAFETY: the caller's guarantee throughout.
    let r = unsafe { umtx_op(obj, op, val, uaddr, uaddr2) };
    match r {
        0 => 0,
        e => fail(e) as c_int,
    }
}

/// # Safety
///
/// As `__sys__umtx_op`.
unsafe fn umtx_op(obj: *mut c_void, op: c_int, val: u64, uaddr: *mut c_void, uaddr2: *mut c_void) -> c_int {
    match op {
        OP_WAIT | OP_WAIT_UINT | OP_WAIT_UINT_PRIVATE => {
            // SAFETY: the caller's guarantee.
            let deadline = match unsafe { deadline(uaddr as usize, uaddr2) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            // A long's low word, on this little-endian machine, or the
            // unsigned int itself.
            // SAFETY: as above.
            unsafe { wait(obj.cast(), val as u32, &deadline) }
        }
        OP_WAKE | OP_WAKE_PRIVATE => {
            futex_wake(obj.cast(), val.min(u32::MAX as u64) as u32);
            0
        }
        OP_NWAKE_PRIVATE => {
            // `obj` is an array of `val` addresses.
            for i in 0..val as usize {
                // SAFETY: the caller's guarantee.
                let a = unsafe { obj.cast::<*const u32>().add(i).read() };
                futex_wake(a, u32::MAX);
            }
            0
        }
        OP_MUTEX_WAIT => {
            // SAFETY: the caller's guarantee.
            let deadline = match unsafe { deadline(uaddr as usize, uaddr2) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            // SAFETY: `obj` is a umutex.
            let m = unsafe { &*obj.cast::<Umutex>() };
            loop {
                let owner = m.m_owner.load(Ordering::Acquire);
                if owner & !UMUTEX_CONTESTED == 0 || owner == UMUTEX_RB_OWNERDEAD || owner == UMUTEX_RB_NOTRECOV {
                    return 0;
                }
                let contested = owner | UMUTEX_CONTESTED;
                if m.m_owner.compare_exchange(owner, contested, Ordering::AcqRel, Ordering::Acquire).is_err() {
                    continue;
                }
                // SAFETY: as above.
                match unsafe { wait_owned(m.m_owner.as_ptr(), contested, &deadline, pi_owner(m, owner)) } {
                    0 => continue,
                    e => return e,
                }
            }
        }
        OP_MUTEX_LOCK | OP_MUTEX_TRYLOCK => {
            // SAFETY: the caller's guarantee.
            let deadline = match unsafe { deadline(uaddr as usize, uaddr2) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            let id = match thr_self() {
                Ok(t) => t as u32,
                Err(e) => return e,
            };
            // SAFETY: `obj` is a umutex.
            let m = unsafe { &*obj.cast::<Umutex>() };
            loop {
                let owner = m.m_owner.load(Ordering::Acquire);
                if owner & !UMUTEX_CONTESTED == 0 {
                    if m.m_owner.compare_exchange(owner, id | (owner & UMUTEX_CONTESTED), Ordering::AcqRel, Ordering::Acquire).is_ok() {
                        return 0;
                    }
                    continue;
                }
                if op == OP_MUTEX_TRYLOCK {
                    return proto::errno::EBUSY;
                }
                let contested = owner | UMUTEX_CONTESTED;
                if m.m_owner.compare_exchange(owner, contested, Ordering::AcqRel, Ordering::Acquire).is_err() {
                    continue;
                }
                // SAFETY: as above.
                match unsafe { wait_owned(m.m_owner.as_ptr(), contested, &deadline, pi_owner(m, owner)) } {
                    0 => continue,
                    // FreeBSD restarts a mutex lock a signal interrupted
                    // (ERESTART in `do_lock_umutex`): again, when the
                    // handler asked for it.
                    e => return e,
                }
            }
        }
        OP_MUTEX_UNLOCK => {
            let id = match thr_self() {
                Ok(t) => t as u32,
                Err(e) => return e,
            };
            // SAFETY: `obj` is a umutex.
            let m = unsafe { &*obj.cast::<Umutex>() };
            loop {
                let owner = m.m_owner.load(Ordering::Acquire);
                if owner & !UMUTEX_CONTESTED != id {
                    return EPERM;
                }
                // Unowned, as libthr's own unlock stores; a waiter it had
                // is woken and sets the bit again if it must wait on.
                if m.m_owner.compare_exchange(owner, UMUTEX_UNOWNED, Ordering::AcqRel, Ordering::Acquire).is_ok() {
                    if owner & UMUTEX_CONTESTED != 0 {
                        wake_one_contested(m);
                    }
                    return 0;
                }
            }
        }
        OP_MUTEX_WAKE | OP_MUTEX_WAKE2 => {
            // SAFETY: `obj` is a umutex.
            let m = unsafe { &*obj.cast::<Umutex>() };
            wake_one_contested(m);
            0
        }
        OP_CV_WAIT => {
            // SAFETY: `obj` is a ucond, `uaddr` the umutex held.
            let cv = unsafe { &*obj.cast::<Ucond>() };
            let flags = val;
            let deadline = if uaddr2.is_null() {
                None
            } else {
                // SAFETY: a timespec, or a `_umtx_time` with CVWAIT_UMTX_TIME.
                let [sec, nsec] = unsafe { uaddr2.cast::<[i64; 2]>().read_unaligned() };
                let (absolute, clock) = if flags & CVWAIT_UMTX_TIME != 0 {
                    // SAFETY: `_umtx_time`'s flags and clock after the time.
                    let [f, c] = unsafe { uaddr2.cast::<u8>().add(16).cast::<[u32; 2]>().read_unaligned() };
                    (f & UMTX_ABSTIME != 0, c as c_int)
                } else {
                    (flags & CVWAIT_ABSTIME != 0, if flags & CVWAIT_CLOCKID != 0 { cv.c_clockid as c_int } else { BSD_CLOCK_REALTIME })
                };
                if !(0..1_000_000_000).contains(&nsec) || sec < 0 {
                    return EINVAL;
                }
                let given = sec as u64 * 1_000_000_000 + nsec as u64;
                let ns = if absolute {
                    given
                } else {
                    match qnx_clock(clock).ok_or(EINVAL).and_then(crate::kercall::clock_time) {
                        Ok(now) => now + given,
                        Err(e) => return e,
                    }
                };
                Some(Deadline { clock, ns })
            };
            let seq = cv.c_seq.load(Ordering::Acquire);
            cv.c_has_waiters.store(1, Ordering::Release);
            // SAFETY: the mutex is held by the caller.
            let unlocked = unsafe { umtx_op(uaddr, OP_MUTEX_UNLOCK, 0, core::ptr::null_mut(), core::ptr::null_mut()) };
            if unlocked != 0 {
                return unlocked;
            }
            // SAFETY: as above.
            match unsafe { wait(cv.c_seq.as_ptr(), seq, &deadline) } {
                e @ (ETIMEDOUT | EINTR) => e,
                _ => 0,
            }
        }
        OP_CV_SIGNAL => {
            // SAFETY: `obj` is a ucond.
            let cv = unsafe { &*obj.cast::<Ucond>() };
            cv.c_seq.fetch_add(1, Ordering::AcqRel);
            let (_, remaining) = futex_wake(cv.c_seq.as_ptr(), 1);
            if remaining == 0 {
                cv.c_has_waiters.store(0, Ordering::Release);
            }
            0
        }
        OP_CV_BROADCAST => {
            // SAFETY: `obj` is a ucond.
            let cv = unsafe { &*obj.cast::<Ucond>() };
            cv.c_seq.fetch_add(1, Ordering::AcqRel);
            cv.c_has_waiters.store(0, Ordering::Release);
            futex_wake(cv.c_seq.as_ptr(), u32::MAX);
            0
        }
        OP_SEM2_WAIT => {
            // SAFETY: the caller's guarantee.
            let deadline = match unsafe { deadline(uaddr as usize, uaddr2) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            // SAFETY: `obj` is a `_usem2`, its count first.
            let count = unsafe { &*obj.cast::<AtomicU32>() };
            loop {
                let c = count.load(Ordering::Acquire);
                if c & !USEM_HAS_WAITERS != 0 {
                    return 0;
                }
                if c != USEM_HAS_WAITERS && count.compare_exchange(c, USEM_HAS_WAITERS, Ordering::AcqRel, Ordering::Acquire).is_err() {
                    continue;
                }
                // SAFETY: as above.
                match unsafe { wait(count.as_ptr(), USEM_HAS_WAITERS, &deadline) } {
                    0 => continue,
                    e => return e,
                }
            }
        }
        OP_SEM2_WAKE => {
            // SAFETY: `obj` is a `_usem2`.
            let count = unsafe { &*obj.cast::<AtomicU32>() };
            if count.load(Ordering::Acquire) & USEM_HAS_WAITERS != 0 {
                futex_wake(count.as_ptr(), 1);
            }
            0
        }
        OP_RW_RDLOCK | OP_RW_WRLOCK => {
            // SAFETY: the caller's guarantee.
            let deadline = match unsafe { deadline(uaddr as usize, uaddr2) } {
                Ok(d) => d,
                Err(e) => return e,
            };
            // SAFETY: `obj` is a urwlock.
            let rw = unsafe { &*obj.cast::<Urwlock>() };
            // SAFETY: as above.
            unsafe { rw_lock(rw, op == OP_RW_RDLOCK, val as u32, &deadline) }
        }
        OP_RW_UNLOCK => {
            // SAFETY: `obj` is a urwlock.
            let rw = unsafe { &*obj.cast::<Urwlock>() };
            rw_unlock(rw)
        }
        OP_ROBUST_LISTS | OP_GET_MIN_TIMEOUT | OP_SET_MIN_TIMEOUT | OP_SET_CEILING => 0,
        OP_SHM => ENOSYS,
        _ => {
            let names = ["lock", "unlock", "wait", "wake", "mutex_trylock", "mutex_lock", "mutex_unlock", "set_ceiling", "cv_wait", "cv_signal", "cv_broadcast", "wait_uint", "rw_rdlock", "rw_wrlock", "rw_unlock", "wait_uint_private", "wake_private", "mutex_wait", "mutex_wake", "sem_wait", "sem_wake", "nwake_private", "mutex_wake2", "sem2_wait", "sem2_wake"];
            not_implemented(names.get(op as usize).copied().unwrap_or("_umtx_op"));
            ENOSYS
        }
    }
}

/// `UMTX_OP_RW_RDLOCK` and `UMTX_OP_RW_WRLOCK`, as `kern_umtx.c`'s
/// `do_rw_rdlock` and `do_rw_wrlock`: take the lock if it is free for
/// the caller (a reader: no writer, and no writer waiting unless
/// readers are preferred; a writer: no owner and no readers), else mark
/// the waiters' bit, count the blocked, and wait on the state word.
/// Every waiter waits on that one word, so an unlock wakes them all and
/// each tries again; FreeBSD keeps two queues and wakes one. The
/// waiters' bits are cleared by the unlock that wakes, never by a
/// leaving waiter: a waiter queued on a state that shows its bit is
/// then always woken, as the bit is cleared only along with a wake (a
/// waiter that comes in between finds it cleared, and sets it again).
///
/// # Safety
///
/// `rw` is the process's.
unsafe fn rw_lock(rw: &Urwlock, read: bool, fflag: u32, deadline: &Option<Deadline>) -> c_int {
    let prefer_reader = fflag & URWLOCK_PREFER_READER != 0 || rw.flags & URWLOCK_PREFER_READER != 0;
    let blocks = |state: u32| -> bool {
        if read {
            let wrflags = URWLOCK_WRITE_OWNER | if prefer_reader { 0 } else { URWLOCK_WRITE_WAITERS };
            state & wrflags != 0
        } else {
            state & URWLOCK_WRITE_OWNER != 0 || state & URWLOCK_MAX_READERS != 0
        }
    };
    let (waiters_bit, blocked) = if read {
        (URWLOCK_READ_WAITERS, &rw.blocked_readers)
    } else {
        (URWLOCK_WRITE_WAITERS, &rw.blocked_writers)
    };
    loop {
        let mut state = rw.state.load(Ordering::Acquire);
        // Free for the caller: take it.
        while !blocks(state) {
            if read && state & URWLOCK_MAX_READERS == URWLOCK_MAX_READERS {
                return EAGAIN;
            }
            let new = if read { state + 1 } else { state | URWLOCK_WRITE_OWNER };
            match rw.state.compare_exchange(state, new, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return 0,
                Err(s) => state = s,
            }
        }
        // Say a waiter is coming, then wait while it blocks.
        while blocks(state) && state & waiters_bit == 0 {
            match rw.state.compare_exchange(state, state | waiters_bit, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => state |= waiters_bit,
                Err(s) => state = s,
            }
        }
        if !blocks(state) {
            continue;
        }
        blocked.fetch_add(1, Ordering::AcqRel);
        let mut err = 0;
        while blocks(state) {
            if state & waiters_bit == 0 {
                // An unlock cleared it meanwhile: again, before waiting.
                match rw.state.compare_exchange(state, state | waiters_bit, Ordering::AcqRel, Ordering::Acquire) {
                    Ok(_) => state |= waiters_bit,
                    Err(s) => state = s,
                }
                continue;
            }
            // SAFETY: the caller's guarantee.
            err = unsafe { wait(rw.state.as_ptr(), state, deadline) };
            if err != 0 {
                break;
            }
            state = rw.state.load(Ordering::Acquire);
        }
        blocked.fetch_sub(1, Ordering::AcqRel);
        if err != 0 {
            return err;
        }
    }
}

/// `UMTX_OP_RW_UNLOCK`, as `do_rw_unlock`: the writer gives the lock up,
/// or a reader leaves; whoever waits is woken to try again, and the
/// waiters' bits go with the wake (see [`rw_lock`]).
fn rw_unlock(rw: &Urwlock) -> c_int {
    let mut state = rw.state.load(Ordering::Acquire);
    loop {
        let new = if state & URWLOCK_WRITE_OWNER != 0 {
            state & !URWLOCK_WRITE_OWNER
        } else if state & URWLOCK_MAX_READERS != 0 {
            state - 1
        } else {
            return EPERM;
        } & !(URWLOCK_WRITE_WAITERS | URWLOCK_READ_WAITERS);
        match rw.state.compare_exchange(state, new, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(s) => state = s,
        }
    }
    if state & (URWLOCK_WRITE_WAITERS | URWLOCK_READ_WAITERS) != 0 {
        futex_wake(rw.state.as_ptr(), u32::MAX);
    }
    0
}

// ANCHOR: wake_one_contested
/// Wakes one waiter of `m`, with the contested bit set first, so that
/// the woken one's unlock wakes the next (FreeBSD's `do_wake2_umutex`
/// sets the bit, then wakes). Set afterwards, the bit could come too
/// late: the woken thread may have locked and unlocked meanwhile, and the
/// rest would sleep on an unowned word for good. When nobody waited, an
/// unowned word's bit is cleared again (a locker that came meanwhile
/// holds the word, and the bit stays: its unlock wakes).
fn wake_one_contested(m: &Umutex) {
    loop {
        let owner = m.m_owner.load(Ordering::Acquire);
        if owner & UMUTEX_CONTESTED != 0 || owner == UMUTEX_RB_OWNERDEAD || owner == UMUTEX_RB_NOTRECOV {
            break;
        }
        if m.m_owner.compare_exchange(owner, owner | UMUTEX_CONTESTED, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            break;
        }
    }
    let (woken, remaining) = futex_wake(m.m_owner.as_ptr(), 1);
    if woken == 0 && remaining == 0 {
        let _ = m.m_owner.compare_exchange(UMUTEX_CONTESTED, UMUTEX_UNOWNED, Ordering::AcqRel, Ordering::Acquire);
    }
}
// ANCHOR_END: wake_one_contested

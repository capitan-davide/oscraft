//! Synchronisation objects (`SyncTypeCreate`, ...).
//!
//! QNX mutexes live in user memory (`sync_t` = { `__count`, `__owner` },
//! `<sys/target_nto.h>`); libc locks them with atomic operations and only
//! calls the kernel when a mutex is contended. `SyncTypeCreate` initialises
//! the `sync_t` from its attributes; the flag bits below are those
//! procnto's `nto_mutex_create` writes.
//!
//! Contention (`SyncMutexLock`, `SyncMutexUnlock`, `SyncCondvarWait`,
//! `SyncCondvarSignal`): the kernel updates `__owner` (offset 4) with
//! atomic operations as libc does (`<sys/inline_mutex.h>`). A held mutex's
//! owner is the holder's TLS `__owner` value, with `_NTO_SYNC_WAITING`
//! (the top bit) set while threads wait; unlocking hands the mutex to the
//! first waiter. Statically initialised objects start at
//! `_NTO_SYNC_INITIALIZER` (-1). Waiters queue per object, keyed by its
//! physical address, so objects in shared memory work across processes.
//!
//! Semaphores (`sem_init()`, `sem_wait()`, `sem_post()`) are all kernel
//! calls: `__count` is the value, `__owner` is `_NTO_SYNC_SEM`.
//! `SyncSemPost` hands a unit straight to the first waiter, if any.
//!
//! Named semaphores (`sem_open()`) are procnto's objects (`NamedSem`, see
//! `proc::namedsem`): their `sync_t` is { the connection to the object,
//! `_NTO_SYNC_NAMED_SEM` }, and `SyncSemWait`, `SyncSemPost` and
//! `SyncCtl(_NTO_SCTL_SEM_VALUE)` act on the object the connection has
//! open. Their waiters queue under the object's (kernel) address.
//!
//! Waits end early with an armed `TimerTimeout` (ETIMEDOUT); a signal
//! wakes a condition variable wait (a spurious wakeup) or ends a semaphore
//! wait (EINTR), not a mutex wait.
//!
//! Read-write locks (`pthread_rwlock_*()`, `struct _sync_rwlock` {
//! `__count`, `__owner` }): libc takes read locks with atomic operations
//! on `__count` (bits 0-23 the readers, bit 31 "a writer holds or waits")
//! and calls `SyncRWLock(rw, op)` for the rest: write locks, read locks
//! while bit 31 is set, the writer's unlock and the last reader's unlock
//! while bit 31 is set. A waiter is woken to retry: its call ends with
//! EINTR, which libc retries. Writers go first.
//!
//! Barriers (`pthread_barrier_wait()`): `__count` is the number of
//! threads, `__owner` `_NTO_SYNC_BARRIER`; `SyncBarrierWait` queues the
//! waiters like the others.
//!
//! Priority-ceiling mutexes (`PTHREAD_PRIO_PROTECT`): `SyncTypeCreate`
//! sets `_NTO_SYNC_KFORCE` in `__count`, so libc locks and unlocks them
//! with kernel calls only, and keeps the ceiling here (`SyncCtl` reads
//! and changes it). The owner runs at the ceiling while it holds one.
//!
//! Priority inheritance (QNX's default protocol: unless `__count` has
//! `_NTO_SYNC_PRIONONE`): a thread that waits for a mutex raises the
//! owner to its priority, and on along a chain of owners that wait for
//! mutexes themselves; the owner goes back when it unlocks (to the
//! priority it had before, or its highest remaining raise) or the waiter
//! gives up. The highest-priority waiter gets the mutex first.
//!
//! Robust mutexes (`PTHREAD_MUTEX_ROBUST`: `SyncTypeCreate` sets
//! `_NTO_SYNC_ROBUST` in `__count`), as procnto (measured):
//! - A process-shared mutex has `_NTO_SYNC_KFORCE` from its creation, so
//!   every lock and unlock comes to the kernel, which knows its owner
//!   (`HELD`). When the owner *process* ends holding a robust one, its
//!   `__owner` becomes `_NTO_SYNC_OWNERDEAD` and its first waiter takes it
//!   with EOWNERDEAD (`process_exited`).
//! - Otherwise the owner's death is found when a thread next locks the
//!   mutex: its `SyncMutexLock` sees an `__owner` naming no thread, takes
//!   the mutex and fails with EOWNERDEAD. Not with an immediate timeout
//!   (libc's trylock): that fails as if the owner lived. And not for a
//!   process-shared mutex whose owner thread ended in a process that
//!   lives on: that is EINVAL.
//! - A mutex taken so is inconsistent: its `__owner` stays
//!   `_NTO_SYNC_OWNERDEAD` (so libc sends its unlock here), the kernel
//!   knows who holds it (`INCONSISTENT`), and other lockers wait.
//!   `SyncCtl(_NTO_SCTL_MUTEX_CONSISTENT)` by that thread makes it
//!   consistent again (EINVAL if it isn't inconsistent, or not its);
//!   unlocked without that, it becomes not recoverable (`__owner`
//!   `_NTO_SYNC_DESTROYED`): every lock and the waiters fail with
//!   ENOTRECOVERABLE.
//! - Threads already waiting when an owner *thread* died are not woken by
//!   its death.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use super::KerResult;
use super::errno::{EAGAIN, EINTR, EINVAL, ENOSYS, ENOTRECOVERABLE, EOWNERDEAD, EPERM, ETIMEDOUT};
use super::umem::{self, Space};
use crate::sched::{self, State, Thread, ThreadRef};
use crate::sync::IrqSpinLock;

const SYNC_MUTEX: u32 = 0;
const SYNC_SEMAPHORE: u32 = 0xffff_fffc;
const SYNC_NAMED_SEM: u32 = 0xffff_fffd;
/// `sync_t.__count` of a fresh mutex.
const MUTEX_INIT: u32 = 0x8000_0000;
/// `_NTO_SYNC_PRIONONE`: no priority inheritance (`PTHREAD_PRIO_NONE`).
const MUTEX_PRIO_NONE: u32 = 0x9000_0000;
/// `_NTO_SYNC_PRIONONE`, `_NTO_SYNC_PRIOCEILING`: a mutex that raises no
/// owner to its waiters' priority.
const NO_INHERIT: u32 = 0x3000_0000;
/// `_NTO_SYNC_PRIOCEILING | _NTO_SYNC_KFORCE` (`PTHREAD_PRIO_PROTECT`).
const MUTEX_PRIO_CEILING: u32 = 0x2400_0000;

/// `SyncTypeCreate(type, sync, attr)`.
pub fn sync_create(ty: u32, sync: u64, attr: u64) -> KerResult {
    let space = Space::current();
    match ty {
        SYNC_MUTEX => {}
        SYNC_SEMAPHORE => return sem_create(space, sync, attr),
        _ => return Err(EINVAL),
    }
    let mut count = MUTEX_INIT;
    if attr != 0 {
        // struct _sync_attr: __protocol, __flags, __prioceiling, ...
        let mut a = [0u8; 32];
        umem::read(space, attr, &mut a)?;
        let field = |i: usize| u32::from_le_bytes(a[i * 4..i * 4 + 4].try_into().unwrap());
        let (protocol, flags) = (field(0), field(1));
        count = match protocol {
            0 => MUTEX_INIT,
            1 => MUTEX_PRIO_NONE,
            2 => {
                let ceiling = field(2);
                if !(1..=255).contains(&ceiling) {
                    return Err(EINVAL);
                }
                let key = umem::object_key(space, sync)?;
                CEILINGS.lock().insert(key, ceiling as u8);
                MUTEX_INIT | MUTEX_PRIO_CEILING
            }
            _ => return Err(EINVAL),
        };
        if flags & 2 != 0 {
            count &= 0x7fff_ffff;
        }
        if flags & 4 != 0 {
            count |= 0x0100_0000;
        }
        if flags & 8 != 0 {
            count |= 0x0800_0000;
        }
        // Process-shared: the kernel sees every lock (procnto's
        // `_NTO_SYNC_SHARED | _NTO_SYNC_KFORCE`, measured).
        if flags & 1 != 0 {
            count |= SHARED | KFORCE;
        }
        if flags & 0x10 != 0 {
            count |= ROBUST;
        }
        let key = umem::object_key(space, sync)?;
        INCONSISTENT.lock().remove(&key);
        HELD.lock().remove(&key);
        a.fill(0);
    }
    let mut init = [0u8; 8];
    init[..4].copy_from_slice(&count.to_le_bytes());
    umem::write(space, sync, &init)?;
    Ok(0)
}

/// `struct _sync_rwlock.__count`: the readers, and "a writer holds or
/// waits".
const RW_READERS: u32 = 0x00ff_ffff;
const RW_WRITER: u32 = 0x8000_0000;
// `SyncRWLock()` operations (`_NTO_RWLOCK_*`)
const RW_READ_LOCK: u32 = 0;
const RW_READ_TRYLOCK: u32 = 1;
const RW_READ_UNLOCK: u32 = 2;
const RW_WRITE_LOCK: u32 = 3;
const RW_WRITE_UNLOCK: u32 = 4;
const RW_WRITE_TRYLOCK: u32 = 5;

/// Threads waiting on read-write locks (by physical address): (thread,
/// a writer).
static RW_WAITERS: IrqSpinLock<BTreeMap<u64, VecDeque<(ThreadRef, bool)>>> =
    IrqSpinLock::new(BTreeMap::new());

/// `SyncRWLock(rw, op)`.
pub fn rwlock(rw: u64, op: u32) -> KerResult {
    let me = sched::current();
    let mine = owner_of(me);
    let space = Space::current();
    let key = umem::object_key(space, rw)?;
    let mut waiters = RW_WAITERS.lock();
    let writers_waiting = waiters.get(&key).is_some_and(|q| q.iter().any(|(_, w)| *w));
    let owner = umem::with_atomic_u32(space, rw + OWNER, |a| a.load(Ordering::Acquire))?;
    let (writer, try_only) = match op {
        RW_READ_LOCK | RW_READ_TRYLOCK => (false, op == RW_READ_TRYLOCK),
        RW_WRITE_LOCK | RW_WRITE_TRYLOCK => (true, op == RW_WRITE_TRYLOCK),
        RW_WRITE_UNLOCK => {
            if owner != mine {
                return Err(EPERM);
            }
            umem::with_atomic_u32(space, rw + OWNER, |a| a.store(0, Ordering::Release))?;
            rw_release(&mut waiters, space, rw, key)?;
            return Ok(0);
        }
        RW_READ_UNLOCK => {
            // libc has counted itself out already.
            let readers =
                umem::with_atomic_u32(space, rw, |a| a.load(Ordering::Acquire))? & RW_READERS;
            if readers == 0 && owner == 0 {
                rw_release(&mut waiters, space, rw, key)?;
            }
            return Ok(0);
        }
        _ => return Err(EINVAL),
    };
    if owner == mine {
        return Err(EDEADLK);
    }
    let got = umem::with_atomic_u32(space, rw, |a| {
        loop {
            let v = a.load(Ordering::Acquire);
            let (new, got) = if writer {
                if v & RW_READERS == 0 && owner == 0 {
                    (v | RW_WRITER, true)
                } else {
                    (v | RW_WRITER, false)
                }
            } else if owner == 0 && !writers_waiting {
                if v & RW_READERS == RW_READERS {
                    return Err(EAGAIN);
                }
                (v + 1, true)
            } else {
                (v | RW_WRITER, false)
            };
            if try_only && !got {
                return Ok(false);
            }
            if a.compare_exchange(v, new, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(got);
            }
        }
    })??;
    if got {
        if writer {
            umem::with_atomic_u32(space, rw + OWNER, |a| a.store(mine, Ordering::Release))?;
        }
        return Ok(0);
    }
    if try_only {
        return Err(EBUSY);
    }
    sched::prepare_block(State::Mutex);
    waiters
        .entry(key)
        .or_default()
        .push_back((me.arc(), writer));
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(State::Mutex);
    drop(waiters);
    sched::block();
    crate::timeout::disarm(t);
    // Woken to try again (EINTR: libc retries), or timed out.
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Err(EINTR),
        e => Err(e),
    }
}

/// Read-write lock `rw` (key `key`) has no writer and no readers any
/// more: wake the first waiting writer, or else every reader, to retry.
/// Call with the waiters' lock held.
fn rw_release(
    waiters: &mut BTreeMap<u64, VecDeque<(ThreadRef, bool)>>,
    space: Space,
    rw: u64,
    key: u64,
) -> Result<(), u32> {
    let queue = waiters.entry(key).or_default();
    let woken: VecDeque<(ThreadRef, bool)> = match queue.iter().position(|(_, w)| *w) {
        Some(i) => queue.remove(i).into_iter().collect(),
        None => core::mem::take(queue),
    };
    let writers_left = queue.iter().any(|(_, w)| *w);
    if queue.is_empty() {
        waiters.remove(&key);
    }
    // Readers may go again on the fast path unless a writer still waits
    // (or is about to retry).
    if !writers_left && !woken.iter().any(|(_, w)| *w) {
        umem::with_atomic_u32(space, rw, |a| a.fetch_and(!RW_WRITER, Ordering::AcqRel))?;
    }
    for (t, _) in woken {
        sched::wake(&t);
    }
    Ok(())
}

/// `SEM_VALUE_MAX`.
const SEM_VALUE_MAX: u32 = 0x3fff_ffff;
use crate::ker::errno::EOVERFLOW;

/// A semaphore: libc's `sem_init()` passes { value, pshared } as `attr`.
fn sem_create(space: Space, sync: u64, attr: u64) -> KerResult {
    let mut a = [0u8; 4];
    if attr != 0 {
        umem::read(space, attr, &mut a)?;
    }
    let value = u32::from_le_bytes(a);
    if value > SEM_VALUE_MAX {
        return Err(EINVAL);
    }
    let mut init = [0u8; 8];
    init[..4].copy_from_slice(&value.to_le_bytes());
    init[4..].copy_from_slice(&SYNC_SEMAPHORE.to_le_bytes());
    umem::write(space, sync, &init)?;
    Ok(0)
}

/// Check that `sync` is a semaphore. Call with the waiters' lock held.
fn is_sem(space: Space, sync: u64) -> Result<(), u32> {
    let owner = umem::with_atomic_u32(space, sync + OWNER, |a| a.load(Ordering::Acquire))?;
    if owner == SYNC_SEMAPHORE {
        Ok(())
    } else {
        Err(EINVAL)
    }
}

/// A named semaphore: its value, changed with the waiters' lock held.
pub struct NamedSem {
    value: AtomicU32,
}

impl NamedSem {
    /// A new one with `value` (EINVAL above `SEM_VALUE_MAX`).
    pub fn new(value: u32) -> Result<Arc<Self>, u32> {
        if value > SEM_VALUE_MAX {
            return Err(EINVAL);
        }
        Ok(Arc::new(NamedSem {
            value: AtomicU32::new(value),
        }))
    }

    pub fn value(&self) -> u32 {
        self.value.load(Ordering::Acquire)
    }

    /// Its waiters' key.
    fn key(self: &Arc<Self>) -> u64 {
        Arc::as_ptr(self) as u64
    }
}

/// The named semaphore the calling process's `sync_t` at `sync` names, if
/// it is one (EINVAL for a connection with none open).
fn named(space: Space, sync: u64) -> Result<Option<Arc<NamedSem>>, u32> {
    let mut b = [0u8; 8];
    umem::read(space, sync, &mut b)?;
    if u32::from_le_bytes(b[4..8].try_into().unwrap()) != SYNC_NAMED_SEM {
        return Ok(None);
    }
    let client = crate::proc::resmgr::Client {
        pid: sched::current().process.pid,
        coid: i32::from_le_bytes(b[0..4].try_into().unwrap()),
    };
    crate::proc::resmgr::named_sem(client).map(Some).ok_or(EINVAL)
}

/// Take a unit of named semaphore `sem`, or wait for one (EAGAIN if
/// `try_only`).
fn named_wait(sem: &Arc<NamedSem>, try_only: bool) -> KerResult {
    let me = sched::current();
    let waiters = WAITERS.lock();
    let v = sem.value.load(Ordering::Acquire);
    if v != 0 {
        sem.value.store(v - 1, Ordering::Release);
        return Ok(0);
    }
    if try_only {
        return Err(EAGAIN);
    }
    sem_block(waiters, me, sem.key())
}

/// Post named semaphore `sem`: wake its first waiter, or add a unit
/// (also `SIGEV_SEM`; callable from interrupt handlers).
pub fn named_post(sem: &Arc<NamedSem>) -> KerResult {
    let mut waiters = WAITERS.lock();
    if let Some(t) = take_waiter(&mut waiters, sem.key()) {
        drop(waiters);
        sched::wake(&t);
        return Ok(0);
    }
    let v = sem.value.load(Ordering::Acquire);
    if v == SEM_VALUE_MAX {
        return Err(EOVERFLOW);
    }
    sem.value.store(v + 1, Ordering::Release);
    Ok(0)
}

/// The first thread waiting under `key`, taken off its queue.
fn take_waiter(waiters: &mut BTreeMap<u64, VecDeque<ThreadRef>>, key: u64) -> Option<ThreadRef> {
    let q = waiters.get_mut(&key)?;
    let next = q.pop_front();
    if q.is_empty() {
        waiters.remove(&key);
    }
    next
}

/// Wait on a semaphore under `key` (the waiters' lock held, released):
/// until a post hands over its unit, a timeout or a signal.
fn sem_block(
    mut waiters: crate::sync::IrqSpinGuard<'_, BTreeMap<u64, VecDeque<ThreadRef>>>,
    me: &Thread,
    key: u64,
) -> KerResult {
    sched::prepare_block(State::Sem);
    waiters.entry(key).or_default().push_back(me.arc());
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(State::Sem);
    drop(waiters);
    sched::block();
    crate::timeout::disarm(t);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Ok(0),
        e => Err(e),
    }
}

// ANCHOR: futex_wait
/// `futex(addr, FUTEX_WAIT, val)` (`personality::native`): wait while the
/// 32-bit word at `addr` holds `val`; EAGAIN at once if it doesn't. The
/// check and the queueing are under the waiters' lock, so a wake after
/// the word changed is not missed. Ends with ETIMEDOUT or EINTR as a
/// semaphore wait does.
pub fn futex_wait(addr: u64, val: u32) -> KerResult {
    use super::errno::EAGAIN;
    let me = sched::current();
    let space = Space::current();
    let key = umem::object_key(space, addr)?;
    let waiters = WAITERS.lock();
    let same = umem::with_atomic_u32(space, addr, |a| a.load(Ordering::Acquire) == val)?;
    if !same {
        return Err(EAGAIN);
    }
    sem_block(waiters, me, key)
}
// ANCHOR_END: futex_wait

/// `futex(addr, FUTEX_WAIT_PI, val, owner)` (`personality::native`): a futex
/// wait that raises thread `owner` (a thread ID of the caller's process,
/// the mutex's holder) to the waiters' priority while they wait, as a
/// QNX mutex raises its owner (`raise`): FreeBSD's PTHREAD_PRIO_INHERIT
/// mutexes. The owner's wake (`futex_wake`) ends its raise by this word;
/// a waiter that leaves early (`unblock`) ends its own part of it.
pub fn futex_wait_pi(addr: u64, val: u32, owner: u32) -> KerResult {
    use super::errno::EAGAIN;
    let me = sched::current();
    let space = Space::current();
    let key = umem::object_key(space, addr)?;
    let waiters = WAITERS.lock();
    let same = umem::with_atomic_u32(space, addr, |a| a.load(Ordering::Acquire) == val)?;
    if !same {
        return Err(EAGAIN);
    }
    sched::prepare_block(State::Sem);
    let mut waiters = waiters;
    waiters.entry(key).or_default().push_back(me.arc());
    if let Some(o) = sched::thread_of(&me.process, owner).filter(|t| t.state() != State::Dead) {
        WAITING_ON.lock().insert(me.tid, (key, owner_of(&o)));
        raise(&waiters, &o, key, top_waiter(&waiters, key));
    }
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(State::Sem);
    drop(waiters);
    sched::block();
    crate::timeout::disarm(t);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Ok(0),
        e => Err(e),
    }
}

/// `futex(addr, FUTEX_WAKE, n)`: wake up to `n` waiters on the word at
/// `addr`; returns how many, and in the high word how many still wait
/// (libsys's mutexes keep the contested bit while some do, as FreeBSD's
/// kernel does from its own count).
pub fn futex_wake(addr: u64, n: u32) -> KerResult {
    let space = Space::current();
    let key = umem::object_key(space, addr)?;
    let mut woken = 0;
    let remaining = loop {
        let mut waiters = WAITERS.lock();
        if woken >= n {
            break waiters.get(&key).map_or(0, |q| q.len()) as u64;
        }
        let Some(t) = take_waiter(&mut waiters, key) else {
            break 0;
        };
        // A waiter that raised its owner (a PI wait): no longer.
        let waited = WAITING_ON.lock().remove(&t.tid);
        if let Some((k, owner)) = waited
            && let Some(o) = owner_thread(owner)
        {
            raise(&waiters, &o, k, top_waiter(&waiters, k));
        }
        drop(waiters);
        sched::wake(&t);
        woken += 1;
    };
    // The caller, if this word raised it, is raised by its waiters left.
    let me = sched::current();
    if RAISED.lock().get(&me.tid).is_some_and(|(_, m)| m.contains_key(&key)) {
        let waiters = WAITERS.lock();
        raise(&waiters, me, key, top_waiter(&waiters, key));
    }
    Ok(u64::from(woken) | (remaining << 32))
}

/// `SyncSemWait(sync, try)`: take a unit, or wait for one (EAGAIN if
/// `try`).
pub fn sem_wait(sync: u64, try_only: bool) -> KerResult {
    let me = sched::current();
    let space = Space::current();
    if let Some(sem) = named(space, sync)? {
        return named_wait(&sem, try_only);
    }
    let key = umem::object_key(space, sync)?;
    let waiters = WAITERS.lock();
    is_sem(space, sync)?;
    let took = umem::with_atomic_u32(space, sync, |a| {
        let v = a.load(Ordering::Acquire);
        if v & SEM_VALUE_MAX == 0 {
            return false;
        }
        a.store(v - 1, Ordering::Release);
        true
    })?;
    if took {
        return Ok(0);
    }
    if try_only {
        return Err(EAGAIN);
    }
    sem_block(waiters, me, key)
}

/// `SyncSemPost(sync)`: wake the first waiter, or add a unit.
pub fn sem_post(sync: u64) -> KerResult {
    let space = Space::current();
    if let Some(sem) = named(space, sync)? {
        return named_post(&sem);
    }
    let key = umem::object_key(space, sync)?;
    let mut waiters = WAITERS.lock();
    is_sem(space, sync)?;
    if let Some(t) = take_waiter(&mut waiters, key) {
        drop(waiters);
        sched::wake(&t);
        return Ok(0);
    }
    umem::with_atomic_u32(space, sync, |a| {
        let v = a.load(Ordering::Acquire);
        if v & SEM_VALUE_MAX == SEM_VALUE_MAX {
            return Err(EOVERFLOW);
        }
        a.store(v + 1, Ordering::Release);
        Ok(())
    })??;
    Ok(0)
}

const WAITING: u32 = 0x8000_0000;
/// `_NTO_SYNC_ROBUST`, `_NTO_SYNC_KFORCE` (in `__count`).
const ROBUST: u32 = 0x0200_0000;
const KFORCE: u32 = 0x0400_0000;
/// `_NTO_SYNC_SHARED` (in `__count`).
const SHARED: u32 = 0x4000_0000;
/// `_NTO_SYNC_WAKEUP`: waiters may be woken (`_NTO_SCTL_MUTEX_WAKEUP`).
const WAKEUP: u32 = 0x0800_0000;
/// `__owner` of a robust mutex whose owner died
/// (`_NTO_SYNC_OWNERDEAD`), while it is inconsistent.
const OWNER_DEAD: u32 = 0xffff_ff00;
const INITIALIZER: u32 = 0xffff_ffff;
const DESTROYED: u32 = 0xffff_fffe;
use crate::ker::errno::EDEADLK;
use crate::ker::errno::EBUSY;
/// Offset of `sync_t.__owner`.
const OWNER: u64 = 4;

/// Threads waiting on each synchronisation object (by physical address).
static WAITERS: IrqSpinLock<BTreeMap<u64, VecDeque<ThreadRef>>> = IrqSpinLock::new(BTreeMap::new());

/// Threads waiting for a mutex that raises its owner, by tid: (mutex key,
/// owner's `__owner` value).
static WAITING_ON: IrqSpinLock<BTreeMap<u32, (u64, u32)>> = IrqSpinLock::new(BTreeMap::new());

/// Owners raised by their mutexes' waiters or ceilings, by tid: (the
/// priority before the first raise, the priority each mutex raises it to;
/// a ceiling under the mutex's key with `CEILING_RAISE` set).
static RAISED: IrqSpinLock<BTreeMap<u32, (u8, BTreeMap<u64, u8>)>> = IrqSpinLock::new(BTreeMap::new());

/// The priority thread `t` runs at when its own priority becomes `prio`
/// (`SchedSet()`, a sporadic thread's budget): the mutexes it holds still
/// raise it above that, and leave it at `prio` when they let go (as
/// procnto: a raised sporadic owner keeps the raise through its budget's
/// end and replenishments).
pub fn own_priority_changed(t: &sched::Thread, prio: u8) -> u8 {
    match RAISED.lock().get_mut(&t.tid) {
        Some(entry) => {
            entry.0 = prio;
            entry.1.values().copied().max().unwrap_or(0).max(prio)
        }
        None => prio,
    }
}

/// Robust mutexes taken from a dead owner and not yet made consistent
/// (by physical address), with the `__owner` value of the thread that
/// holds each.
static INCONSISTENT: IrqSpinLock<BTreeMap<u64, u32>> = IrqSpinLock::new(BTreeMap::new());

/// Process-shared robust mutexes that are held (by physical address): the
/// owner's pid and the mutex's address in its space.
static HELD: IrqSpinLock<BTreeMap<u64, (i32, u64)>> = IrqSpinLock::new(BTreeMap::new());

/// The calling thread now holds mutex `sync` (key `key`): recorded if it
/// is process-shared and robust.
fn took(space: Space, sync: u64, key: u64) -> Result<(), u32> {
    let count = umem::with_atomic_u32(space, sync, |a| a.load(Ordering::Acquire))?;
    if count & (SHARED | ROBUST) == SHARED | ROBUST {
        HELD.lock().insert(key, (sched::current().process.pid, sync));
    }
    Ok(())
}

/// Process `p` (the caller's) ends: the process-shared robust mutexes it
/// holds become `_NTO_SYNC_OWNERDEAD`, each taken by its first waiter,
/// which gets EOWNERDEAD (procnto, measured).
pub fn process_exited(p: &super::process::Process) {
    let space = Space::current();
    let mine: alloc::vec::Vec<(u64, u64)> = {
        let mut held = HELD.lock();
        let keys: alloc::vec::Vec<(u64, u64)> =
            held.iter().filter(|(_, (pid, _))| *pid == p.pid).map(|(k, (_, a))| (*k, *a)).collect();
        for (k, _) in &keys {
            held.remove(k);
        }
        keys
    };
    for (key, sync) in mine {
        let mut waiters = WAITERS.lock();
        let next = waiters.get_mut(&key).and_then(|q| {
            let top = q.iter().map(|t| t.priority()).max()?;
            let i = q.iter().position(|t| t.priority() == top)?;
            q.remove(i)
        });
        if waiters.get(&key).is_some_and(|q| q.is_empty()) {
            waiters.remove(&key);
        }
        if umem::with_atomic_u32(space, sync + OWNER, |a| a.store(OWNER_DEAD, Ordering::Release)).is_err() {
            continue;
        }
        if let Some(t) = next {
            WAITING_ON.lock().remove(&t.tid);
            INCONSISTENT.lock().insert(key, owner_of(&t));
            t.wake_error.store(EOWNERDEAD, Ordering::Relaxed);
            sched::wake(&t);
        }
    }
}

/// The thread an `__owner` value names, if it still runs. The value keeps
/// only 15 bits of the pid, so the process is found by its slot.
fn owner_thread(owner: u32) -> Option<ThreadRef> {
    let owner = owner & !WAITING;
    let p = super::process::by_slot((owner >> 16) as i32)?;
    sched::thread_of(&p, owner & 0xffff).filter(|t| t.state() != State::Dead)
}

/// The highest priority among the waiters of `key`.
fn top_waiter(waiters: &BTreeMap<u64, VecDeque<ThreadRef>>, key: u64) -> Option<u8> {
    waiters.get(&key)?.iter().map(|t| t.priority()).max()
}

// ANCHOR: raise
/// Set how far mutex `key` raises its owner `owner` (None: not at all),
/// then follow the chain: if the owner itself waits for a mutex that
/// raises, that mutex's owner is raised for it. Call with the waiters'
/// lock held.
fn raise(waiters: &BTreeMap<u64, VecDeque<ThreadRef>>, owner: &Thread, key: u64, prio: Option<u8>) {
    let mut owner = owner.arc();
    let (mut key, mut prio) = (key, prio);
    for _ in 0..16 {
        let target = {
            let mut raised = RAISED.lock();
            let entry = raised.entry(owner.tid).or_insert_with(|| (owner.priority(), BTreeMap::new()));
            match prio {
                Some(p) => {
                    entry.1.insert(key, p);
                }
                None => {
                    entry.1.remove(&key);
                }
            }
            let target = entry.1.values().copied().max().unwrap_or(0).max(entry.0);
            if entry.1.is_empty() {
                raised.remove(&owner.tid);
            }
            target
        };
        if target == owner.priority() {
            return;
        }
        sched::set_priority(&owner, target);
        // Waiting for a mutex itself: its owner follows.
        let Some((next_key, next_owner)) = WAITING_ON.lock().get(&owner.tid).copied() else {
            return;
        };
        let Some(next) = owner_thread(next_owner) else {
            return;
        };
        (key, prio, owner) = (next_key, top_waiter(waiters, next_key), next);
    }
}
// ANCHOR_END: raise

/// The priority-ceiling mutexes' ceilings, by physical address.
static CEILINGS: IrqSpinLock<BTreeMap<u64, u8>> = IrqSpinLock::new(BTreeMap::new());

/// A held ceiling mutex's entry in its owner's `RAISED` raises: the
/// mutex's key with this bit set (no physical address has it), apart
/// from the raise its waiters give (none: a ceiling mutex doesn't
/// inherit). Each mutex the thread holds raises it to its ceiling, so
/// the thread runs at the highest ceiling still held, whatever order it
/// unlocks in, and at its own priority when it holds none (procnto,
/// measured).
const CEILING_RAISE: u64 = 1 << 63;

/// Before the calling thread takes mutex `key`: EINVAL if it runs above
/// the mutex's ceiling (POSIX). The ceilings of the mutexes it holds
/// don't count: a thread at 30 under a ceiling of 30 takes a mutex whose
/// ceiling is 20 (procnto, measured).
fn ceiling_check(key: u64) -> Result<(), u32> {
    let Some(ceiling) = CEILINGS.lock().get(&key).copied() else {
        return Ok(());
    };
    let me = sched::current();
    let prio = match RAISED.lock().get(&me.tid) {
        Some((own, raises)) => raises
            .iter()
            .filter(|(k, _)| *k & CEILING_RAISE == 0)
            .map(|(_, p)| *p)
            .max()
            .unwrap_or(0)
            .max(*own),
        None => me.priority(),
    };
    if prio > ceiling { Err(EINVAL) } else { Ok(()) }
}

/// The calling thread has taken mutex `key`: it runs at least at the
/// ceiling.
fn ceiling_enter(key: u64) {
    let Some(ceiling) = CEILINGS.lock().get(&key).copied() else {
        return;
    };
    let waiters = WAITERS.lock();
    raise(&waiters, sched::current(), key | CEILING_RAISE, Some(ceiling));
}

/// The calling thread has released mutex `key`: its ceiling no longer
/// raises it.
fn ceiling_leave(key: u64) {
    let me = sched::current();
    if RAISED.lock().get(&me.tid).is_some_and(|(_, m)| m.contains_key(&(key | CEILING_RAISE))) {
        let waiters = WAITERS.lock();
        raise(&waiters, me, key | CEILING_RAISE, None);
    }
}

/// `SyncCtl(cmd, sync, data)`:
/// - a priority-ceiling mutex's ceiling, changed
///   (`_NTO_SCTL_SETPRIOCEILING`: returns the old one) or read
///   (`_NTO_SCTL_GETPRIOCEILING`). As procnto: a caller that doesn't hold
///   the mutex takes it for the change (waiting for it), and the holder's
///   priority follows the new ceiling;
/// - a robust mutex made consistent (`_NTO_SCTL_MUTEX_CONSISTENT`);
/// - waiters of a `_NTO_SYNC_WAKEUP` mutex woken with EINTR
///   (`_NTO_SCTL_MUTEX_WAKEUP`, `struct _sync_ctl_wakeup` { pid, tid }:
///   that thread, or with tid 0 every waiter of process pid, or of any
///   process with pid 0 too);
/// - a semaphore's value (`_NTO_SCTL_SEM_VALUE`).
pub fn sync_ctl(cmd: u32, sync: u64, data: u64) -> KerResult {
    const SET_PRIOCEILING: u32 = 1;
    const GET_PRIOCEILING: u32 = 2;
    const MUTEX_WAKEUP: u32 = 4;
    const MUTEX_CONSISTENT: u32 = 5;
    const SEM_VALUE: u32 = 6;
    let space = Space::current();
    if cmd == SEM_VALUE {
        return Ok(named(space, sync)?.ok_or(EINVAL)?.value() as u64);
    }
    let key = umem::object_key(space, sync)?;
    match cmd {
        MUTEX_CONSISTENT => {
            // The holder of an inconsistent robust mutex only. Its
            // `__owner` names it again (`_NTO_SYNC_OWNERDEAD`'s top bit
            // left as the waiting bit, as procnto).
            let mine = owner_of(sched::current());
            {
                let mut inconsistent = INCONSISTENT.lock();
                if inconsistent.get(&key) != Some(&mine) {
                    return Err(EINVAL);
                }
                inconsistent.remove(&key);
            }
            umem::with_atomic_u32(space, sync + OWNER, |a| a.store(mine | WAITING, Ordering::Release))?;
            Ok(0)
        }
        SET_PRIOCEILING => {
            let mut b = [0u8; 4];
            umem::read(space, data, &mut b)?;
            let new = i32::from_le_bytes(b);
            if !(1..=255).contains(&new) || !CEILINGS.lock().contains_key(&key) {
                return Err(EINVAL);
            }
            let me = sched::current();
            let owner = umem::with_atomic_u32(space, sync + OWNER, |a| a.load(Ordering::Acquire))?;
            let held = owner & !WAITING == owner_of(me);
            if !held {
                mutex_lock(sync)?;
            }
            let old = {
                let mut ceilings = CEILINGS.lock();
                let c = ceilings.get_mut(&key).ok_or(EINVAL)?;
                core::mem::replace(c, new as u8)
            };
            if held {
                ceiling_enter(key);
            }
            if !held {
                mutex_unlock(sync)?;
            }
            Ok(old as u64)
        }
        MUTEX_WAKEUP => {
            let mut b = [0u8; 8];
            umem::read(space, data, &mut b)?;
            let count = umem::with_atomic_u32(space, sync, |a| a.load(Ordering::Acquire))?;
            if count & WAKEUP == 0 {
                return Err(EINVAL);
            }
            let mut pid = i32::from_le_bytes(b[0..4].try_into().unwrap());
            let tid = u32::from_le_bytes(b[4..8].try_into().unwrap());
            if tid != 0 && pid == 0 {
                pid = sched::current().process.pid;
            }
            let woken: alloc::vec::Vec<ThreadRef> = WAITERS
                .lock()
                .get(&key)
                .map(|q| {
                    q.iter()
                        .filter(|t| pid == 0 || t.process.pid == pid)
                        .filter(|t| tid == 0 || t.ptid == tid)
                        .take(if tid == 0 { usize::MAX } else { 1 })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            for t in woken {
                unblock(&t, super::errno::EINTR);
            }
            Ok(0)
        }
        GET_PRIOCEILING => {
            let prio = *CEILINGS.lock().get(&key).ok_or(EINVAL)?;
            umem::write(space, data, &(prio as i32).to_le_bytes())?;
            Ok(0)
        }
        _ => Err(ENOSYS),
    }
}

/// A thread's `__owner` value (procnto's `thread_bootstrap`).
fn owner_of(t: &Thread) -> u32 {
    ((t.process.pid as u32 & 0x7fff) << 16) | t.ptid
}

enum Lock {
    Got,
    /// From an owner that has gone (a robust mutex).
    OwnerDead,
    Retry,
    /// Held by this `__owner` value.
    Wait(u32),
}

/// `SyncMutexLock(sync)`: wait for a contended mutex.
pub fn mutex_lock(sync: u64) -> KerResult {
    let me = sched::current();
    let mine = owner_of(me);
    let space = Space::current();
    let key = umem::object_key(space, sync)?;
    ceiling_check(key)?;
    loop {
        let mut waiters = WAITERS.lock();
        let count = umem::with_atomic_u32(space, sync, |a| a.load(Ordering::Acquire))?;
        let robust = count & ROBUST != 0;
        let holder = INCONSISTENT.lock().get(&key).copied();
        // libc's trylock: an immediate timeout fails before a dead owner
        // is looked for (procnto).
        let now = crate::timeout::immediate(State::Mutex);
        let r = umem::with_atomic_u32(space, sync + OWNER, |a| {
            let v = a.load(Ordering::Acquire);
            let (new, outcome) = match v {
                DESTROYED if robust => return Err(ENOTRECOVERABLE),
                DESTROYED => return Err(EINVAL),
                INITIALIZER => (mine, Lock::Got),
                OWNER_DEAD if robust => match holder {
                    Some(h) if h == mine => return Err(EDEADLK),
                    Some(h) => (v, Lock::Wait(h)),
                    None => (v, Lock::OwnerDead),
                },
                v if v & !WAITING == 0 => (mine | (v & WAITING), Lock::Got),
                v if v & !WAITING == mine => return Err(EDEADLK),
                v if robust && !now && owner_thread(v).is_none() => {
                    // A shared mutex's owner thread ended, its process
                    // lives on: the kernel never saw it let go.
                    if count & SHARED != 0 && super::process::by_slot(((v & !WAITING) >> 16) as i32).is_some() {
                        return Err(EINVAL);
                    }
                    (OWNER_DEAD, Lock::OwnerDead)
                }
                v => (v | WAITING, Lock::Wait(v)),
            };
            match a.compare_exchange(v, new, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => Ok(outcome),
                Err(_) => Ok(Lock::Retry),
            }
        })??;
        match r {
            Lock::Got => {
                drop(waiters);
                took(space, sync, key)?;
                ceiling_enter(key);
                return Ok(0);
            }
            Lock::OwnerDead => {
                // Ours, inconsistent.
                INCONSISTENT.lock().insert(key, mine);
                drop(waiters);
                took(space, sync, key)?;
                ceiling_enter(key);
                return Err(EOWNERDEAD);
            }
            Lock::Retry => continue,
            Lock::Wait(owner) => {
                sched::prepare_block(State::Mutex);
                waiters.entry(key).or_default().push_back(me.arc());
                let count = umem::with_atomic_u32(space, sync, |a| a.load(Ordering::Acquire))?;
                if count & NO_INHERIT == 0
                    && let Some(o) = owner_thread(owner)
                {
                    WAITING_ON.lock().insert(me.tid, (key, owner));
                    raise(&waiters, &o, key, top_waiter(&waiters, key));
                }
                me.wake_error.store(0, Ordering::Relaxed);
                let t = crate::timeout::arm(State::Mutex);
                drop(waiters);
                // Woken by the unlock that handed the mutex over to us, or
                // by a timeout (then it is not ours).
                sched::block();
                crate::timeout::disarm(t);
                return match me.wake_error.swap(0, Ordering::Relaxed) {
                    0 => {
                        took(space, sync, key)?;
                        ceiling_enter(key);
                        Ok(0)
                    }
                    // Handed over by its owner process's end.
                    EOWNERDEAD => {
                        took(space, sync, key)?;
                        ceiling_enter(key);
                        Err(EOWNERDEAD)
                    }
                    e => Err(e),
                };
            }
        }
    }
}

/// Release the mutex at `sync` (key `key`) held by `mine`: hand it to the
/// first waiter or free it. Call with the waiters' lock held.
fn release(
    waiters: &mut BTreeMap<u64, VecDeque<ThreadRef>>,
    space: Space,
    sync: u64,
    key: u64,
    mine: u32,
) -> Result<(), u32> {
    // Not the owner: EPERM, or EINVAL for a destroyed mutex (procnto's
    // `mutex_unlock`). Checked before a waiter is taken.
    let owner = umem::with_atomic_u32(space, sync + OWNER, |a| a.load(Ordering::Acquire))?;
    if owner & !WAITING != mine {
        return Err(if owner == DESTROYED { EINVAL } else { EPERM });
    }
    HELD.lock().remove(&key);
    let queue = waiters.entry(key).or_default();
    // The highest-priority waiter (the first of them).
    let top = queue.iter().map(|t| t.priority()).max();
    let next = top
        .and_then(|p| queue.iter().position(|t| t.priority() == p))
        .and_then(|i| queue.remove(i));
    let more = !queue.is_empty();
    if queue.is_empty() {
        waiters.remove(&key);
    }
    umem::with_atomic_u32(space, sync + OWNER, |a| {
        let v = a.load(Ordering::Acquire);
        if v & !WAITING != mine {
            return Err(EPERM);
        }
        let new = match &next {
            Some(t) => owner_of(t) | if more { WAITING } else { 0 },
            None => 0,
        };
        a.store(new, Ordering::Release);
        Ok(())
    })??;
    // The old owner is no longer raised by this mutex; the new one is, by
    // the waiters left.
    let me = sched::current();
    if RAISED.lock().get(&me.tid).is_some_and(|(_, m)| m.contains_key(&key)) {
        raise(waiters, me, key, None);
    }
    if let Some(t) = next {
        let inherits = WAITING_ON.lock().remove(&t.tid).is_some();
        if inherits && more {
            raise(waiters, &t, key, top_waiter(waiters, key));
        }
        sched::wake(&t);
    }
    Ok(())
}

/// An inconsistent robust mutex unlocked by its holder: not recoverable
/// from now on (`__owner` `_NTO_SYNC_DESTROYED`, as procnto), and its
/// waiters fail with ENOTRECOVERABLE. Call with the waiters' lock held.
fn give_up(
    waiters: &mut BTreeMap<u64, VecDeque<ThreadRef>>,
    space: Space,
    sync: u64,
    key: u64,
    mine: u32,
) -> Result<(), u32> {
    {
        let mut inconsistent = INCONSISTENT.lock();
        if inconsistent.get(&key) != Some(&mine) {
            return Err(EPERM);
        }
        inconsistent.remove(&key);
    }
    HELD.lock().remove(&key);
    umem::with_atomic_u32(space, sync + OWNER, |a| a.store(DESTROYED, Ordering::Release))?;
    let me = sched::current();
    if RAISED.lock().get(&me.tid).is_some_and(|(_, m)| m.contains_key(&key)) {
        raise(waiters, me, key, None);
    }
    for t in waiters.remove(&key).unwrap_or_default() {
        WAITING_ON.lock().remove(&t.tid);
        t.wake_error.store(ENOTRECOVERABLE, Ordering::Relaxed);
        sched::wake(&t);
    }
    Ok(())
}

/// `SyncMutexUnlock(sync)`: unlock a mutex others wait for.
pub fn mutex_unlock(sync: u64) -> KerResult {
    let me = sched::current();
    let space = Space::current();
    let key = umem::object_key(space, sync)?;
    let mut waiters = WAITERS.lock();
    if INCONSISTENT.lock().contains_key(&key) {
        give_up(&mut waiters, space, sync, key, owner_of(me))?;
        drop(waiters);
        ceiling_leave(key);
        return Ok(0);
    }
    release(&mut waiters, space, sync, key, owner_of(me))?;
    drop(waiters);
    ceiling_leave(key);
    Ok(0)
}

/// `SyncCondvarWait(cond, mutex)`: release the mutex, wait for a signal
/// on the condition variable, take the mutex again.
pub fn condvar_wait(cond: u64, mutex: u64) -> KerResult {
    let me = sched::current();
    let space = Space::current();
    let (ckey, mkey) = (
        umem::object_key(space, cond)?,
        umem::object_key(space, mutex)?,
    );
    let timeout;
    {
        let mut waiters = WAITERS.lock();
        let held = umem::with_atomic_u32(space, mutex + OWNER, |a| {
            a.load(Ordering::Acquire) & !WAITING == owner_of(me)
        })?;
        if !held {
            return Err(EPERM);
        }
        sched::prepare_block(State::Condvar);
        waiters.entry(ckey).or_default().push_back(me.arc());
        me.wake_error.store(0, Ordering::Relaxed);
        // Cannot fail now: the mutex is ours.
        let _ = release(&mut waiters, space, mutex, mkey, owner_of(me));
        timeout = crate::timeout::arm(State::Condvar);
    }
    ceiling_leave(mkey);
    sched::block();
    crate::timeout::disarm(timeout);
    // Why we woke, taken now: re-taking the mutex below may block again
    // (`mutex_lock` clears and sets `wake_error` for its own wait), and
    // a timeout that fired while the signaller still held the mutex
    // must still be reported.
    let woke = me.wake_error.swap(0, Ordering::Relaxed);
    // Take the mutex again: the fast path first, as libc would.
    let got = umem::with_atomic_u32(space, mutex + OWNER, |a| {
        a.compare_exchange(0, owner_of(me), Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
    })?;
    if got {
        took(space, mutex, mkey)?;
        ceiling_enter(mkey);
    } else {
        mutex_lock(mutex)?;
    }
    // A timeout is reported; a signal is a spurious wakeup (allowed).
    match woke {
        ETIMEDOUT => Err(ETIMEDOUT),
        _ => Ok(0),
    }
}

/// Take thread `t`, waiting for a mutex or on a condition variable, off
/// its queue and wake it; its call ends with `err`. Returns whether it was
/// waiting.
pub fn unblock(t: &Thread, err: u32) -> bool {
    {
        let mut rw = RW_WAITERS.lock();
        let mut found = false;
        for q in rw.values_mut() {
            if let Some(i) = q.iter().position(|(w, _)| core::ptr::eq(&**w, t)) {
                q.remove(i);
                found = true;
                break;
            }
        }
        rw.retain(|_, q| !q.is_empty());
        drop(rw);
        if found {
            t.wake_error.store(err, Ordering::Relaxed);
            sched::wake(t);
            return true;
        }
    }
    let mut waiters = WAITERS.lock();
    let mut found = false;
    for q in waiters.values_mut() {
        if let Some(i) = q.iter().position(|w| core::ptr::eq(&**w, t)) {
            q.remove(i);
            found = true;
            break;
        }
    }
    waiters.retain(|_, q| !q.is_empty());
    // A mutex's owner raised for this waiter: as high as the others now.
    let waited = WAITING_ON.lock().remove(&t.tid);
    if let Some((key, owner)) = waited
        && let Some(o) = owner_thread(owner)
    {
        raise(&waiters, &o, key, top_waiter(&waiters, key));
    }
    drop(waiters);
    if found {
        t.wake_error.store(err, Ordering::Relaxed);
        sched::wake(t);
    }
    found
}

/// `sync_t.__owner` of a barrier; `__count` holds the number of threads
/// (and `BARRIER_PSHARED` if it is process-shared).
const SYNC_BARRIER: u32 = 0xffff_fff8;
const BARRIER_COUNT: u32 = 0x3fff_ffff;

/// `SyncBarrierWait(sync)`: wait until the barrier's count of threads
/// waits at it; then all go on, the last to arrive returning -1
/// (`PTHREAD_BARRIER_SERIAL_THREAD`), the others 0. A waiter interrupted
/// by a signal leaves (EINTR; libc waits again).
pub fn barrier_wait(sync: u64) -> KerResult {
    let me = sched::current();
    let space = Space::current();
    let key = umem::object_key(space, sync)?;
    let (count, owner) = umem::with_atomic_u32(space, sync, |a| a.load(Ordering::Acquire))
        .and_then(|c| Ok((c, umem::with_atomic_u32(space, sync + OWNER, |a| a.load(Ordering::Acquire))?)))?;
    if owner != SYNC_BARRIER || count & BARRIER_COUNT == 0 {
        return Err(EINVAL);
    }
    let mut waiters = WAITERS.lock();
    let arrived = waiters.get(&key).map_or(0, |q| q.len()) as u32 + 1;
    if arrived >= count & BARRIER_COUNT {
        let woken = waiters.remove(&key).unwrap_or_default();
        drop(waiters);
        for t in woken {
            t.wake_error.store(0, Ordering::Relaxed);
            sched::wake(&t);
        }
        return Ok(u64::MAX); // -1
    }
    sched::prepare_block(State::Barrier);
    waiters.entry(key).or_default().push_back(me.arc());
    me.wake_error.store(0, Ordering::Relaxed);
    let t = crate::timeout::arm(State::Barrier);
    drop(waiters);
    sched::block();
    crate::timeout::disarm(t);
    match me.wake_error.swap(0, Ordering::Relaxed) {
        0 => Ok(0),
        e => Err(e),
    }
}

/// `SyncCondvarSignal(cond, all)`: wake one waiter, or all.
pub fn condvar_signal(cond: u64, all: bool) -> KerResult {
    // A condition variable's `__owner` is `_NTO_SYNC_COND` (procnto:
    // EINVAL otherwise).
    const SYNC_COND: u32 = 0xffff_fffb;
    let owner = umem::with_atomic_u32(Space::current(), cond + OWNER, |a| a.load(Ordering::Acquire))?;
    if owner != SYNC_COND {
        return Err(EINVAL);
    }
    let key = umem::object_key(Space::current(), cond)?;
    let woken: VecDeque<ThreadRef> = {
        let mut waiters = WAITERS.lock();
        match waiters.get_mut(&key) {
            None => VecDeque::new(),
            Some(q) if all => core::mem::take(q),
            Some(q) => q.pop_front().into_iter().collect(),
        }
    };
    WAITERS.lock().retain(|_, q| !q.is_empty());
    for t in woken {
        sched::wake(&t);
    }
    Ok(0)
}

/// `SyncTypeDestroy(type, sync)` (`pthread_cond_destroy()` & co.): as
/// `SyncDestroy`, EINVAL for an object destroyed already.
pub fn sync_type_destroy(_kind: u32, sync: u64) -> KerResult {
    let owner = umem::with_atomic_u32(Space::current(), sync + OWNER, |a| {
        a.load(Ordering::Acquire)
    })?;
    if owner == DESTROYED {
        return Err(EINVAL);
    }
    sync_destroy(sync)
}

/// `SyncDestroy(sync)`: EBUSY while threads wait on it.
pub fn sync_destroy(sync: u64) -> KerResult {
    let space = Space::current();
    let key = umem::object_key(space, sync)?;
    if WAITERS.lock().get(&key).is_some_and(|q| !q.is_empty()) {
        return Err(EBUSY);
    }
    umem::with_atomic_u32(space, sync + OWNER, |a| {
        a.store(DESTROYED, Ordering::Release)
    })?;
    CEILINGS.lock().remove(&key);
    INCONSISTENT.lock().remove(&key);
    HELD.lock().remove(&key);
    Ok(0)
}

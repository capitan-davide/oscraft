//! The FreeBSD personality, OsCraft's native one. Its kernel calls
//! (`__KER_OSCRAFT_*`, `ker::number`): what FreeBSD's libthr needs of a
//! kernel and QNX's calls don't give.
//!
//! - `thr_new(param)`: FreeBSD's thread creation (`<sys/thr.h>`): a
//!   thread on the stack the caller allocated, its thread pointer the
//!   caller's TLS block, started at `start_func(arg)`, its ID written to
//!   `child_tid` and `parent_tid` before either runs. QNX's
//!   `ThreadCreate()` builds QNX's thread-local block at the top of the
//!   stack and starts libc's entry instead.
//! - `thr_self()`: the calling thread's ID, which QNX's libc reads from
//!   that block.
//! - `futex(addr, op, val, owner)`: wait while the 32-bit word at `addr`
//!   holds `val` (raising thread `owner` meanwhile, for a mutex that
//!   inherits priority), or wake up to `val` waiters on it. Waiters queue with the
//!   synchronisation objects' (`sync::WAITERS`, by physical address), so
//!   an armed `TimerTimeout()` (`_NTO_TIMEOUT_SEM`) ends a wait with
//!   ETIMEDOUT and a signal with EINTR. libsys builds `_umtx_op()`'s
//!   operations on it, as a libc builds them on Linux's futex.

use core::sync::atomic::Ordering;

use super::Personality;
use crate::ker::KerResult;
use crate::ker::errno::{EINVAL, ENOSYS};
use crate::ker::umem::{self, Space};
use crate::sched;

/// `struct thr_param` (`<sys/thr.h>`): 8-byte fields.
const THR_PARAM_SIZE: usize = 0x68;
const FUTEX_WAIT: u32 = 0;
const FUTEX_WAKE: u32 = 1;
const FUTEX_WAIT_PI: u32 = 2;
/// A FreeBSD process's thread IDs start here, above `PID_MAX`, as in
/// FreeBSD: libthr writes them in mutexes' owner words, where small numbers
/// are special (`UMUTEX_RB_OWNERDEAD`, `UMUTEX_RB_NOTRECOV`: 16 and 17).
const TID_BASE: u64 = 100_000;

/// OsCraft's extension to `struct _msg_info.flags`: the sender is a
/// FreeBSD process, whose messages carry FreeBSD's layouts. Bit 15 is
/// unused by QNX's `_NTO_MI_*` (`<sys/neutrino.h>`, SDP 8.0.5). A QNX
/// client's word stays exactly procnto's (msg2test checks it), which is
/// why the native side carries the bit.
pub const MI_NATIVE: u16 = 0x8000;

/// Only a FreeBSD process may make these calls.
fn check_personality() -> Result<(), u32> {
    if sched::current().process.personality() == Personality::Native {
        Ok(())
    } else {
        Err(ENOSYS)
    }
}

/// `thr_new(param, size)`, with the scheduling of FreeBSD's
/// `thr_param.rtp` as libsys converted it: `policy` (`SchedSet()`'s
/// number; 0: the caller's, with its priority) and `priority`. FreeBSD
/// creates the thread with them (`kern_thr_new()`), and so does this: a
/// thread started at its creator's priority and changed afterwards may
/// already wait for a mutex by then, having lent its holder the wrong
/// priority (test `thr-test-pi`).
pub fn thr_new(param: u64, size: u64, policy: u32, priority: i64) -> KerResult {
    check_personality()?;
    if size as usize != THR_PARAM_SIZE {
        return Err(EINVAL);
    }
    let named = if policy == 0 { None } else { Some(crate::ker::new_thread_sched(policy, priority)?) };
    let space = Space::current();
    let mut p = [0u8; THR_PARAM_SIZE];
    umem::read(space, param, &mut p)?;
    let at = |o: usize| u64::from_le_bytes(p[o..o + 8].try_into().unwrap());
    let (start_func, arg) = (at(0), at(8));
    let (stack_base, stack_size) = (at(0x10), at(0x18));
    let tls_base = at(0x20);
    let (child_tid, parent_tid) = (at(0x30), at(0x38));
    let me = sched::current();
    // The ID is written where the parent and the child look for it
    // before the child exists, as FreeBSD does: the child may run on
    // another CPU as soon as it is made, and reads its ID at once.
    let ptid = me.process.new_ptid();
    let id = (u64::from(ptid) + TID_BASE).to_le_bytes();
    for tid in [child_tid, parent_tid] {
        if tid != 0 && let Err(e) = umem::write(space, tid, &id) {
            me.process.release_ptid(ptid);
            return Err(e);
        }
    }
    crate::ker::thread::start(
        &me.process,
        stack_base,
        stack_size,
        start_func,
        arg,
        tls_base,
        named.map_or(me.base_priority(), |(_, prio)| prio),
        me.sigmask.load(Ordering::Relaxed),
        ptid,
        |t| t.set_policy(named.map_or(me.policy(), |(policy, _)| policy)),
    )?;
    Ok(0)
}

/// `thr_self()`: the calling thread's ID.
pub fn thr_self() -> KerResult {
    check_personality()?;
    Ok(u64::from(sched::current().ptid) + TID_BASE)
}

/// `futex(addr, op, val, owner)`.
pub fn futex(addr: u64, op: u32, val: u32, owner: u32) -> KerResult {
    check_personality()?;
    match op {
        FUTEX_WAIT => crate::ker::sync::futex_wait(addr, val),
        FUTEX_WAKE => crate::ker::sync::futex_wake(addr, val),
        FUTEX_WAIT_PI => crate::ker::sync::futex_wait_pi(addr, val, owner),
        _ => Err(EINVAL),
    }
}

/// FreeBSD's `struct stat` (`<sys/stat.h>`), the system's: what process
/// 1's managers build and a native process gets as it is.
pub use proto::stat::{FORM_FREEBSD as STAT_FORM_FREEBSD, Stat};

/// A `struct stat` as bytes.
pub fn stat_bytes(st: &Stat) -> [u8; 224] {
    // SAFETY: a plain repr(C) struct of 224 bytes with no padding.
    unsafe { core::mem::transmute_copy(st) }
}

/// FreeBSD's `struct dirent` for a name: `d_fileno`, `d_off`, `d_reclen`,
/// `d_type` (unknown), `d_namlen`, the name with its NUL at 24, padded to
/// 8 bytes.
pub fn dirent(ino: u64, off: u64, name: &str) -> alloc::vec::Vec<u8> {
    let reclen = (24 + name.len() + 1).next_multiple_of(8);
    let mut r = alloc::vec![0u8; reclen];
    r[0..8].copy_from_slice(&ino.to_le_bytes());
    r[8..16].copy_from_slice(&off.to_le_bytes());
    r[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
    r[20..22].copy_from_slice(&(name.len() as u16).to_le_bytes());
    r[24..24 + name.len()].copy_from_slice(name.as_bytes());
    r
}

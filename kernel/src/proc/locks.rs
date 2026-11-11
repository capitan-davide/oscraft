//! POSIX advisory record locks (`fcntl()` `F_GETLK`, `F_SETLK`,
//! `F_SETLKW`; `_IO_LOCK`), for procnto's files (`/dev/shmem`, so `/tmp`
//! and `/var`), keyed by file.
//!
//! - A lock is a byte range (`l_len` 0: to the end of the file, and
//!   beyond) of one process, shared (`F_RDLCK`) or exclusive (`F_WRLCK`).
//!   Two conflict when they overlap, belong to different processes and
//!   one is exclusive.
//! - `F_SETLK` sets or clears (`F_UNLCK`) a range: EAGAIN on a conflict,
//!   else the process's own locks in the range are replaced.
//!   `F_SETLKW` doesn't wait yet: it is EAGAIN on a conflict too.
//! - `F_GETLK` reports a conflicting lock, or `F_UNLCK` in `l_type`.
//! - A process's locks on a file go when it closes any descriptor of it
//!   (`release`), all of them when it ends (`process_exited`).
//! - `flock()` (whole-file BSD locks) comes as `F_FLOCK` (0x80) in
//!   `l_type`, with `LOCK_SH` 1, `LOCK_EX` 4 and `LOCK_UN` 3 below it. Its
//!   locks should belong to the open file, not the process: here they are
//!   the process's too.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::ker::errno::{EAGAIN, EINVAL};
use crate::sync::IrqSpinLock;

const F_SETLK: u32 = 6;
const F_SETLKW: u32 = 7;
const F_GETLK: u32 = 14;
const F_RDLCK: i16 = 1;
const F_WRLCK: i16 = 2;
const F_UNLCK: i16 = 3;
/// `flock()`'s exclusive lock, with `F_FLOCK`.
const FLOCK_EX: i16 = 4;
const F_FLOCK: i16 = 0x80;
/// `flock64_t`: l_type, l_whence, l_zero1, l_start, l_len, l_pid, l_sysid.
pub const FLOCK_SIZE: usize = 32;

#[derive(Clone, Copy)]
struct Lock {
    pid: i32,
    /// [start, end), end u64::MAX: to the end and beyond.
    start: u64,
    end: u64,
    exclusive: bool,
}

static LOCKS: IrqSpinLock<BTreeMap<u64, Vec<Lock>>> = IrqSpinLock::new(BTreeMap::new());

/// `_IO_LOCK` (`struct _io_lock` { type, combine_len, subtype (the
/// command), nbytes } then a `flock64_t`) from process `pid` on file `key`
/// whose position is `pos` and size `size`; returns the reply data.
pub fn lock(key: u64, pid: i32, msg: &[u8], pos: u64, size: u64) -> Result<Vec<u8>, u32> {
    let cmd = msg
        .get(4..8)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or(EINVAL)?
        // `<fcntl.h>` numbers the lock commands `_F_OFF` (100) above their
        // base on this ABI (F_GETLK 114, F_SETLK 106...), and the 64-bit
        // forms the same (`_F_LF64_OFF` 0): the base number is the rest.
        % 100;
    let fl: &[u8; FLOCK_SIZE] = msg
        .get(12..12 + FLOCK_SIZE)
        .ok_or(EINVAL)?
        .try_into()
        .unwrap();
    let ty = match i16::from_le_bytes([fl[0], fl[1]]) & !F_FLOCK {
        FLOCK_EX => F_WRLCK,
        t => t,
    };
    let whence = i16::from_le_bytes([fl[2], fl[3]]);
    let start = i64::from_le_bytes(fl[8..16].try_into().unwrap());
    let len = i64::from_le_bytes(fl[16..24].try_into().unwrap());
    let base = match whence {
        0 => 0,
        1 => pos as i64,
        2 => size as i64,
        _ => return Err(EINVAL),
    };
    let from = base.checked_add(start).ok_or(EINVAL)?;
    let (from, to) = match len {
        0 => (from, None),
        l if l > 0 => (from, Some(from.checked_add(l).ok_or(EINVAL)?)),
        l => (from.checked_add(l).ok_or(EINVAL)?, Some(from)),
    };
    if from < 0 {
        return Err(EINVAL);
    }
    let want = Lock {
        pid,
        start: from as u64,
        end: to.map_or(u64::MAX, |t| t as u64),
        exclusive: ty == F_WRLCK,
    };
    if !matches!(ty, F_RDLCK | F_WRLCK | F_UNLCK) {
        return Err(EINVAL);
    }
    let mut all = LOCKS.lock();
    let locks = all.entry(key).or_default();
    let conflict = locks.iter().find(|l| {
        l.pid != pid && l.start < want.end && want.start < l.end && (l.exclusive || want.exclusive)
    });
    match cmd {
        F_GETLK => {
            let mut out = [0u8; FLOCK_SIZE];
            match conflict {
                Some(l) if ty != F_UNLCK => {
                    let ty = if l.exclusive { F_WRLCK } else { F_RDLCK };
                    out[0..2].copy_from_slice(&ty.to_le_bytes());
                    out[8..16].copy_from_slice(&(l.start as i64).to_le_bytes());
                    let len = if l.end == u64::MAX {
                        0
                    } else {
                        l.end - l.start
                    };
                    out[16..24].copy_from_slice(&(len as i64).to_le_bytes());
                    out[24..28].copy_from_slice(&l.pid.to_le_bytes());
                }
                _ => {
                    out.copy_from_slice(fl);
                    out[0..2].copy_from_slice(&F_UNLCK.to_le_bytes());
                }
            }
            // struct _io_lock_reply { zero[3] } then the flock64_t.
            let mut reply = alloc::vec![0u8; 12];
            reply.extend_from_slice(&out);
            Ok(reply)
        }
        F_SETLK | F_SETLKW => {
            if ty != F_UNLCK && conflict.is_some() {
                return Err(EAGAIN);
            }
            // Our locks in the range go (the parts outside it stay).
            let mut kept = Vec::with_capacity(locks.len() + 1);
            for l in locks.drain(..) {
                if l.pid != pid || l.end <= want.start || want.end <= l.start {
                    kept.push(l);
                    continue;
                }
                if l.start < want.start {
                    kept.push(Lock {
                        end: want.start,
                        ..l
                    });
                }
                if want.end < l.end {
                    kept.push(Lock {
                        start: want.end,
                        ..l
                    });
                }
            }
            if ty != F_UNLCK {
                kept.push(want);
            }
            *locks = kept;
            if locks.is_empty() {
                all.remove(&key);
            }
            Ok(Vec::new())
        }
        _ => Err(EINVAL),
    }
}

/// Process `pid` closed a descriptor of file `key`: its locks on it go.
pub fn release(key: u64, pid: i32) {
    let mut all = LOCKS.lock();
    if let Some(locks) = all.get_mut(&key) {
        locks.retain(|l| l.pid != pid);
        if locks.is_empty() {
            all.remove(&key);
        }
    }
}

/// Process `pid` has ended: all its locks go.
pub fn process_exited(pid: i32) {
    let mut all = LOCKS.lock();
    for locks in all.values_mut() {
        locks.retain(|l| l.pid != pid);
    }
    all.retain(|_, l| !l.is_empty());
}

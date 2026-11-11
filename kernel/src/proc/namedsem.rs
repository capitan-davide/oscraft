//! Named semaphores (`sem_open()`, `sem_unlink()`), as procnto (checked
//! with semtest in the reference image).
//!
//! libc's `sem_open()` sends an `_IO_CONNECT` of `_FTYPE_SEM` for the name
//! as it is (the path manager sends those here), with
//! `_IO_CONNECT_EXTRA_SEM` (the initial value, 4 bytes) when it may create
//! the semaphore; the connection then names it (`sync::NamedSem`).
//! `O_CREAT` makes a missing one (a value above `SEM_VALUE_MAX`: EINVAL),
//! `O_EXCL` fails on an existing one (EEXIST); without `O_CREAT` a missing
//! one is ENOENT. `SEM_ANON` is the name `__anon__` with `O_ANON`: a new
//! semaphore with no name. `sem_unlink()` (`_IO_CONNECT_UNLINK`) removes
//! the name; those open keep the semaphore.
//!
//! The names show in `/dev/sem`: `S_IFNAM` with the creation mode,
//! `st_rdev` `S_INSEM`, and the value as `st_size`.
//!
//! Opening an existing one takes read and write permission in the class of
//! its mode that applies to the caller (owner, else group or a
//! supplementary group, else others), EACCES otherwise; root may open any.
//! Anyone may unlink one. As procnto (measured).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use super::pathmgr::Connect;
use crate::ker::errno::{EACCES, EEXIST, EINVAL, ENOENT};
use crate::ker::process::Cred;
use crate::ker::sync::NamedSem;
use crate::sync::IrqSpinLock;

/// `_FTYPE_SEM`
pub const FTYPE_SEM: u32 = 8;
/// `_IO_CONNECT_EXTRA_SEM`
const EXTRA_SEM: u8 = 6;
const O_CREAT: u32 = 0x200;
const O_EXCL: u32 = 0x800;
/// `O_ANON` (`sem_open(SEM_ANON)`)
const O_ANON: u32 = 0x2000;
/// `S_IFNAM`, and `S_INSEM` for `st_rdev`.
pub const S_IFNAM: u32 = 0o050000;
pub const S_INSEM: u32 = 1;

/// A semaphore with what `stat()` shows of it.
#[derive(Clone)]
pub struct Sem {
    pub sem: Arc<NamedSem>,
    pub ino: u32,
    /// `S_IFNAM` and the permissions.
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
}

static NAMES: IrqSpinLock<BTreeMap<String, Sem>> = IrqSpinLock::new(BTreeMap::new());
static NEXT_INO: AtomicU32 = AtomicU32::new(0x6000_0000);

/// Open (or create) the semaphore of connect `c` for a process with
/// `umask` and effective IDs `uid`, `gid`.
pub fn open(c: &Connect, umask: u32, cred: &Cred) -> Result<Sem, u32> {
    let (uid, gid) = (cred.euid, cred.egid);
    let name = c.path.trim_matches('/');
    let create = |c: &Connect| -> Result<Sem, u32> {
        let value = if c.extra_type == EXTRA_SEM && c.combined.len() >= 4 {
            u32::from_le_bytes(c.combined[..4].try_into().unwrap())
        } else {
            0
        };
        Ok(Sem {
            sem: NamedSem::new(value)?,
            ino: NEXT_INO.fetch_add(1, Ordering::Relaxed),
            mode: S_IFNAM | (c.mode & 0o777 & !umask),
            uid,
            gid,
        })
    };
    if c.ioflag & O_ANON != 0 {
        return create(c);
    }
    if name.is_empty() || name.contains('/') {
        return Err(EINVAL);
    }
    let mut names = NAMES.lock();
    if let Some(s) = names.get(name) {
        if c.ioflag & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL {
            return Err(EEXIST);
        }
        if !may_open(s, cred) {
            return Err(EACCES);
        }
        return Ok(s.clone());
    }
    if c.ioflag & O_CREAT == 0 {
        return Err(ENOENT);
    }
    let s = create(c)?;
    names.insert(name.into(), s.clone());
    Ok(s)
}

/// Whether `cred` may open semaphore `s`: read and write permission in the
/// class of its mode that applies (the first that matches), or root.
fn may_open(s: &Sem, cred: &Cred) -> bool {
    const RW: u32 = 0o6;
    let shift = if cred.euid == 0 {
        return true;
    } else if cred.euid == s.uid {
        6
    } else if cred.egid == s.gid || cred.groups.contains(&s.gid) {
        3
    } else {
        0
    };
    (s.mode >> shift) & RW == RW
}

/// The semaphore named `name`, for `stat()` through `/dev/sem`.
pub fn lookup(name: &str) -> Option<Sem> {
    NAMES.lock().get(name.trim_matches('/')).cloned()
}

/// `sem_unlink()`: remove the name.
pub fn unlink(name: &str) -> Result<(), u32> {
    NAMES.lock().remove(name.trim_matches('/')).map(|_| ()).ok_or(ENOENT)
}

/// The names in `/dev/sem` with their inodes (no `.` and `..`, as
/// procnto).
pub fn names() -> Vec<(u64, String)> {
    NAMES.lock().iter().map(|(n, s)| (s.ino as u64, n.clone())).collect()
}

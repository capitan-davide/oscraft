//! Shared memory objects at `/dev/shmem` (QNX `memmgr/devmem.c`,
//! `shmem_*`): `shm_open()` opens `/dev/shmem/<name>`, and QNX's images
//! link `/tmp` there; OsCraft's `/tmp` is a `tmpfs` of its own.
//!
//! Contract, from the SDP's procnto (`shmem_open`, `shmem_unlink`,
//! `mem_stat`, `mem_read_write`, `mem_space`, `mem_lseek`):
//! - Open: names up to 1023 bytes (else ENAMETOOLONG). An existing object
//!   opens, unless `O_CREAT | O_EXCL` (EEXIST); a missing one is created
//!   empty with `O_CREAT` (and file type any, file or shmem), else ENOENT.
//!   `SHM_ANON` (`O_ANON`, FreeBSD-form 0x2000) creates an object without a name. The
//!   empty path is the directory. Names may have several components
//!   (`slogger2/console.9`, as `slogger2` makes them): their leading
//!   components are directories, which exist while a name is below them.
//! - No `mknod()`: no directories, sockets' nodes or FIFOs (ENOSYS), as
//!   procnto, whose `shmem_connect_funcs` have only open, unlink and mount
//!   (read with REA). Those are a file system's: `/tmp` and `/var` are
//!   `tmpfs` (user/tmpfs).
//! - Directories open for reading only (EISDIR); a plain `read()` of one
//!   reads nothing, `readdir()` (`_IO_XTYPE_READDIR`) the names.
//! - Unlink removes the name (ENOENT if none); the object lives on while
//!   it is open or mapped.
//! - Stat: an object made by `shm_open()` (`_FTYPE_SHMEM`) is `S_IFNAM`
//!   with `st_rdev` `S_INSHD`, one made by `open()` (a file of `/tmp`)
//!   `S_IFREG` (procnto's `do_devmem_open`), with its permissions, its
//!   size, as many links as references, no blocks, on device 1;
//!   `/dev/shmem` and the directories that exist for the names below them
//!   are `S_IFDIR | 01777`, with 2 links, on device 0x407. Times are 0
//!   unless `utime()` gives one: procnto's shared memory has no clock.
//! - Read and write need the open's access (EBADF). They copy at the file
//!   position (which moves), at the end with `O_APPEND`, or at the offset
//!   of an `_IO_XTYPE_OFFSET` (pread()/pwrite(), position unchanged); a
//!   write grows the object, a read stops at its end.
//! - `_IO_SPACE` (ftruncate(), posix_fallocate()): `start` from `whence`
//!   (negative: ENOMEM). `F_GROWSP`/`F_ALLOCSP` grow the object to
//!   `start + len`; `F_FREESP` cuts it at `start` when the range reaches
//!   its end (EINVAL for a hole, or without write access). The reply is
//!   the size.
//! - lseek: SEEK_SET/CUR/END, EOVERFLOW below 0.
//! - `_IO_ACL` on an object: setting or checking an ACL succeeds
//!   (everyone is root), getting one gets none; a directory has none
//!   (ENOSYS).
//!
//! - `rename()` (`_IO_CONNECT_RENAME` at the new name, the old one in the
//!   extra, both relative to `/dev/shmem`): a name replaces another
//!   name (EISDIR over a directory); a made directory moves with what is
//!   below it (ENOTEMPTY over a directory that has names, ENOTDIR over a
//!   name, EINVAL into itself). The new name's directory must exist
//!   (ENOENT).
//!
//! Not yet: links, mapping permission checks, memory limits.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::imagefs::{Entry, S_IFDIR};
use super::pathmgr::Connect;
use super::resmgr;
use crate::ker::errno::{
    EBADF, EBADMSG, EEXIST, EINVAL, ENAMETOOLONG, ENOENT, ENOMEM, ENOSYS, EPERM,
};
use crate::sync::IrqSpinLock;
use crate::vm::object::MemObject;
use crate::vm::pmem::PAGE_SIZE;

/// `S_IFNAM` and its shared-memory subtype `S_INSHD` (`<sys/stat.h>`).
const S_IFNAM: u32 = 0x5000;
const S_INSHD: u32 = 2;
const S_IFREG: u32 = 0x8000;
const NAME_MAX: usize = 1023;
// open flags in `ioflag` (<fcntl.h>), `_IO_FLAG_*`
const IO_FLAG_RD: u32 = 0x1;
const IO_FLAG_WR: u32 = 0x2;
const O_APPEND: u32 = 0x8;
pub const O_CREAT: u32 = 0x200;
const O_TRUNC: u32 = 0x400;
const O_EXCL: u32 = 0x800;
const SHM_ANON: u32 = 0x2000;
// file types (<sys/ftype.h>)
const FTYPE_ANY: u32 = 0;
const FTYPE_FILE: u32 = 1;
const FTYPE_SHMEM: u32 = 5;
// I/O messages (<sys/iomsg.h>)
const IO_READ: u16 = 0x101;
const IO_WRITE: u16 = 0x102;
const IO_STAT: u16 = 0x104;
const IO_LSEEK: u16 = 0x109;
const IO_CHMOD: u16 = 0x10a;
const IO_CHOWN: u16 = 0x10b;
const IO_UTIME: u16 = 0x10c;
const IO_SPACE: u16 = 0x110;
const IO_ACL: u16 = 0x11b;
const IO_READ64: u16 = 0x11e;
// _IO_ACL subtypes
const ACL_GET: u32 = 0;
const ACL_SET: u32 = 1;
const ACL_CHK: u32 = 2;
const IO_WRITE64: u16 = 0x11f;
const IO_UTIME64: u16 = 0x121;
const XTYPE_MASK: u32 = 0xff;
const XTYPE_NONE: u32 = 0;
const XTYPE_OFFSET: u32 = 5;
const XTYPE_READDIR: u32 = 9;
// _IO_SPACE subtypes (F_GROWSP, F_ALLOCSP, F_FREESP; <fcntl.h>)
const F_GROWSP: u16 = 109;
const F_ALLOCSP: u16 = 110;
const F_FREESP: u16 = 111;
use crate::ker::errno::EOVERFLOW;
const INO_BASE: u32 = 0x4000_0000;

/// A shared memory object and what `stat()` says of it.
impl ShmFile {
    /// Its creator, and whether it is revocable (`SHMCTL_REVOCABLE`).
    pub fn creator(&self) -> i32 {
        self.creator
    }

    pub fn revocable(&self) -> bool {
        self.ctl.load(Ordering::Relaxed) & SHMCTL_REVOCABLE != 0
    }
}

pub struct ShmFile {
    pub obj: Arc<MemObject>,
    /// The process that made it (`SHMCTL_SEAL`, `SHMCTL_REVOCABLE`).
    creator: i32,
    /// Its `shm_ctl()` flags (`SHMCTL_*`) and special value (with bit 31
    /// once set).
    ctl: AtomicU32,
    special: AtomicU32,
    /// The typed memory pool `SHMCTL_TYMEM` took its memory from (its
    /// name): any more must come from the same one.
    tymem: IrqSpinLock<Option<String>>,
    ino: u32,
    /// Permission bits.
    mode: AtomicU32,
    /// The owner, as `chown()` leaves it.
    uid: AtomicU32,
    gid: AtomicU32,
    /// Last change, in seconds since the epoch.
    mtime: AtomicU64,
    /// Its file type: `S_IFNAM` or `S_IFREG`, for a
    /// Unix-domain socket's name (not an object).
    ftype: u32,
}

/// The contents and permission bits of the file `name` (relative to
/// `/dev/shmem`), if it is a regular file (one made by `open()`, as a
/// program copied to `/tmp`): for the loader.
pub fn contents(name: &str) -> Option<(Vec<u8>, u32)> {
    let f = NAMES.lock().get(name)?.clone();
    if f.ftype != S_IFREG {
        return None;
    }
    // A kernel buffer the size of the file, for the loader: bounded, as
    // `ftruncate()` sets the size without frames and a program is small
    // (review item PATHMEM-5; a buffer past the heap's means is a panic).
    const MAX_PROGRAM: u64 = 64 << 20;
    let size = f.obj.size();
    if size > MAX_PROGRAM || f.obj.backed_bytes() > MAX_PROGRAM {
        return None;
    }
    let mut data = alloc::vec![0u8; size as usize];
    let n = f.obj.read(0, &mut data).ok()?;
    data.truncate(n);
    Some((data, f.mode.load(Ordering::Relaxed)))
}

/// The names in `/dev/shmem`.
static NAMES: IrqSpinLock<BTreeMap<String, Arc<ShmFile>>> = IrqSpinLock::new(BTreeMap::new());
static NEXT_INO: AtomicU32 = AtomicU32::new(INO_BASE + 1);

/// What an open of `/dev/shmem` refers to.
#[derive(Clone)]
pub enum Open {
    /// A directory (`path` relative to `/dev/shmem`, "" for itself);
    /// `pos` counts the names read.
    Dir { path: String, pos: u64, ioflag: u32 },
    File {
        /// None: created with `SHM_ANON`.
        name: Option<String>,
        file: Arc<ShmFile>,
        pos: u64,
        ioflag: u32,
    },
}

/// What procnto's shared memory takes for "now" (creation, writes,
/// `utime()` without a time): 0; only a time given is kept (measured).
fn now_secs() -> u64 {
    0
}

/// `st_dev` of `/dev/shmem`'s directories and of its objects (measured).
const DIR_DEV: u32 = 0x407;
const OBJECT_DEV: u32 = 0x1;

/// Who `pid` acts as: effective user and group.
fn owner_of(pid: i32) -> (u32, u32) {
    crate::ker::process::lookup(pid).map_or((0, 0), |p| {
        let c = p.cred.lock();
        (c.euid, c.egid)
    })
}

/// Whether `pid` may open `file` as `ioflag` asks: the class of its mode
/// that applies, the first that matches, or root (procnto, measured).
fn may_open(file: &ShmFile, pid: i32, ioflag: u32) -> bool {
    let Some(p) = crate::ker::process::lookup(pid) else {
        return true;
    };
    let cred = p.cred.lock();
    let (uid, gid) = (file.uid.load(Ordering::Relaxed), file.gid.load(Ordering::Relaxed));
    let shift = if cred.euid == 0 {
        return true;
    } else if cred.euid == uid {
        6
    } else if cred.egid == gid || cred.groups.contains(&gid) {
        3
    } else {
        0
    };
    let bits = (file.mode.load(Ordering::Relaxed) >> shift) & 7;
    let wanted = if ioflag & IO_FLAG_RD != 0 { 4 } else { 0 } | if ioflag & IO_FLAG_WR != 0 { 2 } else { 0 };
    bits & wanted == wanted
}

fn new_file(mode: u32, ftype: u32, creator: i32) -> Arc<ShmFile> {
    let (uid, gid) = owner_of(creator);
    Arc::new(ShmFile {
        obj: Arc::new(MemObject::new()),
        creator,
        ctl: AtomicU32::new(0),
        special: AtomicU32::new(0),
        tymem: IrqSpinLock::new(None),
        ino: NEXT_INO.fetch_add(1, Ordering::Relaxed),
        mode: AtomicU32::new(mode & 0o7777),
        uid: AtomicU32::new(uid),
        gid: AtomicU32::new(gid),
        mtime: AtomicU64::new(now_secs()),
        ftype,
    })
}


/// Open `c.path` (relative to `/dev/shmem`) for process `pid`, whose file
/// creation mask is `umask`.
pub fn open(c: &Connect, umask: u32, pid: i32) -> Result<Open, u32> {
    let ioflag = c.ioflag;
    let mode = c.mode & !umask;
    if ioflag & SHM_ANON != 0 {
        let file = new_file(mode, S_IFNAM, pid);
        return Ok(Open::File {
            name: None,
            file,
            pos: 0,
            ioflag,
        });
    }
    let name = c.path.trim_matches('/');
    if name.len() > NAME_MAX {
        return Err(ENAMETOOLONG);
    }
    if name
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
        && !name.is_empty()
    {
        return Err(ENOENT);
    }
    let mut names = NAMES.lock();
    if name.is_empty() || (!names.contains_key(name) && is_dir(&names, name)) {
        if ioflag & O_CREAT != 0 && ioflag & O_EXCL != 0 {
            return Err(EEXIST);
        }
        // Directories are opened for reading only (procnto).
        if ioflag & IO_FLAG_WR != 0 {
            return Err(crate::ker::errno::EISDIR);
        }
        let path = name.into();
        return Ok(Open::Dir {
            path,
            pos: 0,
            ioflag,
        });
    }
    let file = match names.get(name) {
        Some(_) if ioflag & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL => return Err(EEXIST),
        Some(f) if !may_open(f, pid, ioflag) => return Err(crate::ker::errno::EACCES),
        Some(f) => f.clone(),
        None => {
            if ioflag & O_CREAT == 0 || !matches!(c.file_type, FTYPE_ANY | FTYPE_FILE | FTYPE_SHMEM)
            {
                return Err(ENOENT);
            }
            let ftype = if c.file_type == FTYPE_SHMEM {
                S_IFNAM
            } else {
                S_IFREG
            };
            let f = new_file(mode, ftype, pid);
            names.insert(name.into(), f.clone());
            f
        }
    };
    drop(names);
    // A socket node opens for writing only: `libsocket`'s `connect()` opens
    // it so, and the stack requires write access of the node it is handed
    // (io-sock's `verify_handle`, read with REA).
    if ioflag & O_TRUNC != 0 && ioflag & IO_FLAG_WR != 0 {
        file.obj.set_size(0)?;
    }
    Ok(Open::File {
        name: Some(name.into()),
        file,
        pos: 0,
        ioflag,
    })
}

/// Remove name `path` (relative to `/dev/shmem`).
pub fn unlink(path: &str) -> Result<(), u32> {
    let path = path.trim_matches('/');
    let mut names = NAMES.lock();
    // The last reference frees the object's memory: not under the lock
    // (it masks interrupts; review PATHMEM-4).
    if let Some(gone) = names.remove(path) {
        drop(names);
        drop(gone);
        return Ok(());
    }
    Err(ENOENT)
}

/// `rename(old, new)`, both relative to `/dev/shmem`.
pub fn rename(old: &str, new: &str) -> Result<(), u32> {
    use crate::ker::errno::EISDIR;
    let (old, new) = (old.trim_matches('/'), new.trim_matches('/'));
    if old.is_empty() || new.is_empty() {
        return Err(EINVAL);
    }
    if new.len() > NAME_MAX {
        return Err(ENAMETOOLONG);
    }
    let mut names = NAMES.lock();
    if let Some((parent, _)) = new.rsplit_once('/')
        && !is_dir(&names, parent)
    {
        return Err(ENOENT);
    }
    if old == new {
        return if names.contains_key(old) || is_dir(&names, old) {
            Ok(())
        } else {
            Err(ENOENT)
        };
    }
    if let Some(f) = names.remove(old) {
        if is_dir(&names, new) {
            names.insert(old.into(), f);
            return Err(EISDIR);
        }
        names.insert(new.into(), f);
        return Ok(());
    }
    Err(ENOENT)
}

/// Is a name below `dir`?
fn below(names: &BTreeMap<String, Arc<ShmFile>>, dir: &str) -> bool {
    let prefix = alloc::format!("{dir}/");
    names
        .range(prefix.clone()..)
        .next()
        .is_some_and(|(n, _)| n.starts_with(&prefix))
}

/// Is `dir` (a path below `/dev/shmem`) a directory: a name below it?
fn is_dir(names: &BTreeMap<String, Arc<ShmFile>>, dir: &str) -> bool {
    below(names, dir)
}

/// The inode of directory `dir`.
fn dir_ino(dir: &str) -> u32 {
    if dir.is_empty() {
        return INO_BASE;
    }
    // FNV-1a, in a range of its own below the objects'.
    let h = dir.bytes().fold(0x811c_9dc5u32, |h, b| {
        (h ^ b as u32).wrapping_mul(0x0100_0193)
    });
    0x3000_0000 | (h & 0x0fff_ffff)
}

/// The names in directory `dir`, with their inodes.
fn names(dir: &str) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> =
        alloc::vec![(dir_ino(dir) as u64, ".".into()), (1, "..".into())];
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        alloc::format!("{dir}/")
    };
    let mut subdirs = BTreeSet::new();
    for (n, f) in NAMES.lock().iter().filter(|(n, _)| n.starts_with(&prefix)) {
        let rest = &n[prefix.len()..];
        match rest.split_once('/') {
            // A directory below: listed once.
            Some((sub, _)) => {
                subdirs.insert(String::from(sub));
            }
            None => out.push((f.ino as u64, rest.into())),
        }
    }
    for sub in subdirs {
        out.push((dir_ino(&alloc::format!("{prefix}{sub}")) as u64, sub));
    }
    out
}

/// The file of an open, for record locks (`locks`): its key, the open's
/// position and the file's size.
pub fn lock_target(o: &Open) -> Option<(u64, u64, u64)> {
    match o {
        Open::File { file, pos, .. } => Some((1 << 32 | file.ino as u64, *pos, file.obj.size())),
        Open::Dir { .. } => None,
    }
}

/// The path and open flags `iofdinfo()` reports for an open: `/dev/shmem`
/// itself is "memory", with only its access mode (procnto's).
pub fn fdinfo_path(o: &Open) -> (String, u32) {
    match o {
        Open::Dir { path, ioflag, .. } if path.is_empty() => ("memory".into(), *ioflag & 3),
        _ => (self::path(o), ioflag(o)),
    }
}

/// How many named objects there are (`/proc/vm/stats`).
pub fn count() -> usize {
    NAMES.lock().len()
}

/// The path of an open.
pub fn path(o: &Open) -> String {
    match o {
        Open::File { name: Some(n), .. } => alloc::format!("/dev/shmem/{n}"),
        Open::Dir { path, .. } if !path.is_empty() => alloc::format!("/dev/shmem/{path}"),
        _ => "/dev/shmem".into(),
    }
}

pub fn ioflag(o: &Open) -> u32 {
    match o {
        Open::Dir { ioflag, .. } | Open::File { ioflag, .. } => *ioflag,
    }
}

/// Whether a descriptor with `ioflag` may be mapped with `prot`, shared
/// or private: `mmap()` is EACCES for a descriptor not open for reading,
/// and for `PROT_WRITE` with `MAP_SHARED` on one not open for writing
/// (QNX mmap(), Errors; measured, PATHMEM-3). A private writable mapping
/// of a read-only descriptor is allowed: the writes stay in the copy.
pub fn map_allowed(ioflag: u32, prot: u32, shared: bool) -> Result<(), u32> {
    if ioflag & IO_FLAG_RD == 0 {
        return Err(crate::ker::errno::EACCES);
    }
    if shared && prot & crate::vm::map::PROT_WRITE != 0 && ioflag & IO_FLAG_WR == 0 {
        return Err(crate::ker::errno::EACCES);
    }
    Ok(())
}

/// Whether a descriptor with `ioflag` is open for writing.
pub fn writable(ioflag: u32) -> bool {
    ioflag & IO_FLAG_WR != 0
}

/// Change an open's `ioflag` (`fcntl(F_SETFL)`).
pub fn with_ioflag(o: &mut Open, f: impl FnOnce(&mut u32)) {
    match o {
        Open::Dir { ioflag, .. } | Open::File { ioflag, .. } => f(ioflag),
    }
}

/// The memory object an open refers to (for `mmap()`).
pub fn file(o: &Open) -> Option<Arc<ShmFile>> {
    match o {
        Open::File { file, .. } => Some(file.clone()),
        Open::Dir { .. } => None,
    }
}

/// The `struct stat` of an open.
pub fn stat(o: &Open) -> resmgr::StatReply {
    let (ino, mode, nlink, size, mtime, rdev) = match o {
        Open::Dir { path, .. } => {
            (dir_ino(path), S_IFDIR | 0o1777, 2, 0, now_secs(), 0)
        }
        // A regular file has no subtype.
        Open::File { file, .. } if file.ftype == S_IFREG => (
            file.ino,
            file.ftype | file.mode.load(Ordering::Relaxed),
            Arc::strong_count(file) as u32,
            file.obj.size(),
            file.mtime.load(Ordering::Relaxed),
            0,
        ),
        Open::File { file, .. } => (
            file.ino,
            file.ftype | file.mode.load(Ordering::Relaxed),
            Arc::strong_count(file) as u32,
            file.obj.size(),
            file.mtime.load(Ordering::Relaxed),
            S_INSHD,
        ),
    };
    let (uid, gid) = match o {
        Open::File { file, .. } => (file.uid.load(Ordering::Relaxed), file.gid.load(Ordering::Relaxed)),
        _ => (0, 0),
    };
    let e = Entry {
        ino,
        mode,
        uid,
        gid,
        mtime: mtime as u32,
        data: &[],
    };
    let mut s = resmgr::stat(&e);
    s.st.st_size = size as i64;
    s.st.st_rdev = u64::from(rdev);
    s.st.st_nlink = u64::from(nlink);
    let dev = if matches!(o, Open::Dir { .. }) { DIR_DEV } else { OBJECT_DEV };
    s.st.st_dev = u64::from(dev);
    s.extra.blocksize = PAGE_SIZE as u32;
    s.st.st_blocks = 0;
    s
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// An I/O message on an open of `/dev/shmem`: the reply's status and data.
// shm_ctl() flags (<sys/mman.h>)
const SHMCTL_ANON: u32 = 0x1;
const SHMCTL_PHYS: u32 = 0x2;
const SHMCTL_SEAL: u32 = 0x4;
const SHMCTL_REVOCABLE: u32 = 0x8;
const SHMCTL_LAZY: u32 = 0x400;
const SHMCTL_REPEAT: u32 = 0x800;
const SHMCTL_TYMEM: u32 = 0x2000;
const SHMCTL_GET_FLAGS: u32 = 0x4000;
/// `shm_ctl_special()`: the special value applies.
const SHMCTL_SPECIAL: u32 = 0x8000;
/// Flags `shm_ctl()` refuses (EINVAL): those outside `SHMCTL_FLAG_MASK`
/// (0xffff), and 0x100, the one bit below it `<sys/mman.h>` defines no
/// flag or hint for.
const SHMCTL_UNKNOWN: u32 = !0xffff | 0x100;

/// `shm_ctl()` and `shm_ctl_special()` (`DCMD_MEMMGR_CTRL`, the data { paddr,
/// size, flags, special }) on object `o` for process `pid`, as procnto's
/// `shmem_devctl`; the devctl's result:
/// - EINVAL without flags, for unknown ones, or `SHMCTL_TYMEM` without
///   `SHMCTL_ANON`. `SHMCTL_GET_FLAGS` alone reads the flags (the result).
/// - A sealed object (`SHMCTL_SEAL`, which only its creator sets) changes
///   no more (EPERM).
/// - An object that has memory takes no other flags or special value
///   (EBADF); the same flags again may grow it.
/// - `SHMCTL_ANON`: memory up to the file position plus `size` (never
///   shrinking), allocated now unless `SHMCTL_LAZY`; contiguous with
///   `SHMCTL_PHYS`, which can't grow a non-empty object (EBUSY) nor be
///   lazy (EINVAL).
/// - `SHMCTL_ANON | SHMCTL_TYMEM`: the memory comes from the typed memory
///   pool open on descriptor `paddr` (`tymem`, which the caller looks
///   up): one opened with `POSIX_TYPED_MEM_ALLOCATE`, or with
///   `_ALLOCATE_CONTIG` and `SHMCTL_PHYS`, and always the same pool
///   (EBADF otherwise). `SHMCTL_TYMEM` isn't kept in the flags.
/// - `SHMCTL_PHYS` alone: the physical memory at `paddr` (page-aligned,
///   needs `PROCMGR_AID_MEM_PHYS`), at the end of the object.
/// - `SHMCTL_SPECIAL` (`shm_ctl_special()`) needs `PROCMGR_AID_MEM_SPECIAL`
///   (EPERM).
/// - `SHMCTL_REPEAT` (`paddr` a stride, `size` a count): the physical run
///   at the file position, `len` bytes, is followed by `count` more of
///   `len` bytes, each `stride` further on in physical memory, where the
///   object has no memory yet; the object ends after the last.
/// - Such an object can't be truncated (`_IO_SPACE`: EINVAL).
/// Not yet: revocation.
pub fn shm_ctl(o: &Open, data: &[u8], pid: i32, tymem: Option<super::tymem::Open>) -> Result<u32, u32> {
    let Open::File { file, pos, .. } = o else {
        return Err(EINVAL);
    };
    if data.len() < 24 {
        return Err(EINVAL);
    }
    let (paddr, size) = (u64_at(data, 0), u64_at(data, 8));
    let (flags, special) = (u32_at(data, 16), u32_at(data, 20));
    if flags == 0 || flags & SHMCTL_UNKNOWN != 0 || flags & (SHMCTL_TYMEM | SHMCTL_ANON) == SHMCTL_TYMEM {
        return Err(EINVAL);
    }
    let old = file.ctl.load(Ordering::Relaxed);
    let mut want = flags;
    if want & SHMCTL_GET_FLAGS != 0 {
        want &= !SHMCTL_GET_FLAGS;
        if want == 0 {
            return Ok(old & 0xffff);
        }
    }
    let result = if flags & SHMCTL_GET_FLAGS != 0 { old & 0xffff } else { 0 };
    // shm_ctl_special() takes PROCMGR_AID_MEM_SPECIAL (as procnto).
    if want & SHMCTL_SPECIAL != 0
        && !crate::ker::process::lookup(pid)
            .is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::MEM_SPECIAL, None))
    {
        return Err(EPERM);
    }
    if old & SHMCTL_SEAL != 0 {
        return Err(EPERM);
    }
    let layout_only = want & !(SHMCTL_SEAL | SHMCTL_REVOCABLE) == 0;
    if want & (SHMCTL_SEAL | SHMCTL_REVOCABLE) != 0 && pid != file.creator {
        return Err(EPERM);
    }
    let has_memory = file.obj.size() > 0 || old & (SHMCTL_ANON | SHMCTL_PHYS) != 0;
    if want & SHMCTL_REVOCABLE != 0 {
        if !layout_only && (want & (SHMCTL_ANON | SHMCTL_PHYS) == 0 || want & SHMCTL_LAZY != 0) {
            return Err(EINVAL);
        }
        if layout_only && !(has_memory && old & (SHMCTL_ANON | SHMCTL_PHYS) != 0 && old & SHMCTL_LAZY == 0) {
            return Err(EINVAL);
        }
    }
    if layout_only {
        file.ctl.fetch_or(want, Ordering::Relaxed);
        return Ok(result);
    }
    // The flags and special value asked for, and what changes.
    if want & SHMCTL_REPEAT != 0 {
        want = (want & !(SHMCTL_REPEAT | SHMCTL_PHYS)) | SHMCTL_PHYS;
    }
    let mut asked = want & !SHMCTL_TYMEM;
    let new_special = if want & SHMCTL_SPECIAL != 0 {
        if special > 0x3fff_ffff {
            return Err(EINVAL);
        }
        special | 0x8000_0000
    } else {
        asked |= old & SHMCTL_SPECIAL;
        file.special.load(Ordering::Relaxed)
    };
    let changes = (old & 0xffff) ^ asked;
    if has_memory
        && (changes & !(SHMCTL_SEAL | SHMCTL_REVOCABLE) != 0 || file.special.load(Ordering::Relaxed) != new_special)
    {
        return Err(EBADF);
    }
    file.ctl.fetch_or(changes, Ordering::Relaxed);
    file.special.store(new_special, Ordering::Relaxed);
    let undo = |e: u32| {
        file.ctl.fetch_and(!changes, Ordering::Relaxed);
        if changes != 0 {
            file.special.store(0, Ordering::Relaxed);
        }
        Err(e)
    };
    if flags & (SHMCTL_ANON | SHMCTL_LAZY) != 0 {
        file.ctl.fetch_or(SHMCTL_ANON, Ordering::Relaxed);
        if flags & (SHMCTL_LAZY | SHMCTL_PHYS) == SHMCTL_LAZY | SHMCTL_PHYS || flags & SHMCTL_REPEAT != 0 {
            return undo(EINVAL);
        }
        let cur = file.obj.size();
        let end = pos.saturating_add(size);
        if end <= cur {
            return Ok(result);
        }
        if cur != 0 && flags & SHMCTL_PHYS != 0 {
            return undo(crate::ker::errno::EBUSY);
        }
        if flags & SHMCTL_TYMEM != 0 {
            let contig = flags & SHMCTL_PHYS != 0;
            let Some(ty) = tymem.filter(|t| {
                let tflag = t.tflag & (super::tymem::ALLOCATE | super::tymem::ALLOCATE_CONTIG);
                tflag == if contig { super::tymem::ALLOCATE_CONTIG } else { super::tymem::ALLOCATE }
            }) else {
                return undo(EBADF);
            };
            {
                let mut pool = file.tymem.lock();
                match pool.as_deref() {
                    Some(name) if name != ty.name => return undo(EBADF),
                    Some(_) => {}
                    None => *pool = Some(ty.name.clone()),
                }
            }
            return match tymem_grow(&file.obj, cur, end, &ty, contig) {
                Ok(()) => Ok(result),
                Err(e) => undo(e),
            };
        }
        let r = if flags & SHMCTL_PHYS != 0 {
            // Contiguous RAM, the object's; a size with no room in the
            // arithmetic is ENOMEM (as procnto, measured, PATHMEM-6).
            let Some(len) = end.checked_next_multiple_of(PAGE_SIZE) else {
                return undo(ENOMEM);
            };
            match crate::vm::pmem::alloc_contig(len / PAGE_SIZE, 0, u64::MAX) {
                Some(pa) => {
                    for off in (0..len).step_by(PAGE_SIZE as usize) {
                        crate::vm::kmem::with_frame(pa + off, |f| f.fill(0));
                    }
                    file.obj.add_frames(0, pa, len, true).and_then(|()| file.obj.set_size(end))
                }
                // The event goes out once `resmgr::io` has let go of the
                // context's lock (review PATHMEM-2).
                None => {
                    super::procevent::trigger_later(super::procevent::CONTIG_ALLOC_FAIL, 0);
                    Err(ENOMEM)
                }
            }
        } else {
            // Backed at once, SHMCTL_LAZY or not: procnto takes the memory
            // now for an object that isn't typed memory (measured).
            file.obj.grow(end)
        };
        return match r {
            Ok(()) => Ok(result),
            Err(e) => undo(e),
        };
    }
    if flags & SHMCTL_REPEAT != 0 {
        return match repeat(&file.obj, *pos, paddr, size, pid) {
            Ok(()) => Ok(result),
            Err(e) => undo(e),
        };
    }
    if flags & SHMCTL_PHYS == 0 {
        return Ok(result);
    }
    // SHMCTL_PHYS alone: the memory at paddr, at the end of the object;
    // what there is of it (`phys_end`), and a size of 0 is nothing to
    // check (accepted, measured). procnto lets a range that wraps around
    // the top of the address range through (measured); here it is EINVAL
    // too (`add_frames`).
    let aligned = (paddr | size) % PAGE_SIZE == 0;
    let within = size == 0
        || paddr.checked_add(size - 1).is_some_and(|last| last <= crate::vm::pmem::phys_end());
    if flags & (SHMCTL_LAZY | SHMCTL_TYMEM) != 0 || !aligned || !within || file.obj.size() > *pos {
        return undo(EINVAL);
    }
    let allowed = crate::ker::process::lookup(pid).is_some_and(|p| {
        crate::ker::ability::allowed(&p, crate::ker::ability::MEM_PHYS, Some(paddr))
    });
    if !allowed {
        return undo(EPERM);
    }
    match file.obj.add_frames(*pos, paddr, size, false) {
        Ok(()) => Ok(result),
        Err(e) => undo(e),
    }
}

/// Grow `obj` from `cur` to `end` bytes with zeroed memory from typed
/// memory pool `ty` (one run if `contig`). All of it or nothing, as
/// procnto (measured, review PATHMEM-15): a pool, or a kernel reserve,
/// that can't spare it is ENOMEM before anything is taken, and the object
/// stays as it was.
fn tymem_grow(obj: &MemObject, cur: u64, end: u64, ty: &super::tymem::Open, contig: bool) -> Result<(), u32> {
    use crate::vm::pmem;
    let from = cur.next_multiple_of(PAGE_SIZE);
    let pages = (end.next_multiple_of(PAGE_SIZE) - from) / PAGE_SIZE;
    if pages > 0 && (!pmem::can_spare(pages) || pmem::free_in(ty.lo, ty.hi) < pages * PAGE_SIZE) {
        return Err(ENOMEM);
    }
    // Scattered memory comes in runs as long as the pool has them (up to
    // 512 pages, halved when there is none), not a page at a time: each
    // allocation scans the pool from its start, with interrupts masked.
    let mut runs: Vec<(u64, u64)> = Vec::new();
    let give_back = |runs: &[(u64, u64)]| {
        for &(pa, n) in runs {
            (0..n).for_each(|i| pmem::free_page(pa + i * PAGE_SIZE));
        }
    };
    let (mut left, mut want) = (pages, if contig { pages } else { pages.min(512) });
    while left > 0 {
        let n = want.min(left);
        match pmem::alloc_contig(n, ty.lo, ty.hi) {
            Some(pa) => {
                runs.push((pa, n));
                left -= n;
            }
            None if !contig && want > 1 => want /= 2,
            None => {
                give_back(&runs);
                return Err(ENOMEM);
            }
        }
    }
    let mut at = from;
    for &(pa, n) in &runs {
        for off in (0..n * PAGE_SIZE).step_by(PAGE_SIZE as usize) {
            crate::vm::kmem::with_frame(pa + off, |f| f.fill(0));
        }
        if let Err(e) = obj.add_frames(at, pa, n * PAGE_SIZE, true) {
            // The runs already added come out again, the size back to
            // what it was: the object as it was (only the frame table's
            // growth can fail here).
            obj.take_frames(from, at - from, cur);
            give_back(&runs);
            return Err(e);
        }
        at += n * PAGE_SIZE;
    }
    obj.set_size(end)
}

/// `SHMCTL_REPEAT` (procnto's checks): `count` copies of the physical run
/// at `pos`, `stride` apart.
fn repeat(obj: &MemObject, pos: u64, stride: u64, count: u64, pid: i32) -> Result<(), u32> {
    let count = u32::try_from(count).ok().filter(|&c| c != 0).ok_or(EINVAL)? as u64;
    if stride == 0 || stride as i64 <= 0 || stride % PAGE_SIZE != 0 {
        return Err(EINVAL);
    }
    let (base, len) = obj.phys_run(pos).ok_or(EINVAL)?;
    let first = base.checked_add(stride).ok_or(EINVAL)?;
    let end = count.checked_mul(stride).and_then(|n| n.checked_add(first)).ok_or(EINVAL)?;
    if len > stride {
        return Err(EINVAL);
    }
    let allowed = crate::ker::process::lookup(pid).is_some_and(|p| {
        crate::ker::ability::allowed(&p, crate::ker::ability::MEM_PHYS, Some(first))
    });
    if !allowed {
        return Err(EPERM);
    }
    let mut at = pos + len;
    for pa in (first..end).step_by(stride as usize) {
        obj.fill_frames(at, pa, len)?;
        at += len;
    }
    Ok(())
}

pub fn io(rcvid: u64, client: i32, ty: u16, msg: &[u8], total: usize, o: &mut Open) -> Result<(u32, Vec<u8>), u32> {
    match (ty, o) {
        (IO_STAT, o) => Ok(stat(o).encode(client)),
        (IO_READ | IO_READ64, Open::Dir { path, pos, .. }) if msg.len() >= 16 => {
            let xtype = u32_at(msg, 8) & XTYPE_MASK;
            // A plain read() or a pread() of a directory reads nothing
            // (procnto).
            if xtype == XTYPE_NONE || xtype == XTYPE_OFFSET {
                return Ok((0, Vec::new()));
            }
            if xtype != XTYPE_READDIR {
                return Err(ENOSYS);
            }
            let (out, n) = resmgr::dirents(client, &names(path), *pos as usize, u32_at(msg, 4) as usize);
            *pos += n as u64;
            Ok((out.len() as u32, out))
        }
        (
            IO_READ | IO_READ64,
            Open::File {
                file, pos, ioflag, ..
            },
        ) if msg.len() >= 16 => {
            if *ioflag & IO_FLAG_RD == 0 {
                return Err(EBADF);
            }
            let mut nbytes = u32_at(msg, 4) as u64;
            if ty == IO_READ64 {
                nbytes |= (u32_at(msg, 12) as u64) << 32;
            }
            let at = match u32_at(msg, 8) & XTYPE_MASK {
                XTYPE_NONE => None,
                XTYPE_OFFSET if msg.len() >= 24 => Some(u64_at(msg, 16)),
                _ => return Err(ENOSYS),
            };
            let from = at.unwrap_or(*pos);
            let len = nbytes.min(file.obj.size().saturating_sub(from)) as usize;
            let (n, data) = resmgr::read_out(rcvid, len, |off, piece| file.obj.read(from + off as u64, piece))?;
            if at.is_none() {
                *pos = from + n as u64;
            }
            Ok((n as u32, data))
        }
        (
            IO_WRITE | IO_WRITE64,
            Open::File {
                file, pos, ioflag, ..
            },
        ) if msg.len() >= 16 => {
            if *ioflag & IO_FLAG_WR == 0 {
                return Err(EBADF);
            }
            let nbytes = u32_at(msg, 4) as usize;
            let (at, data_at) = match u32_at(msg, 8) & XTYPE_MASK {
                XTYPE_NONE if *ioflag & O_APPEND != 0 => (file.obj.size(), 16),
                XTYPE_NONE => (*pos, 16),
                XTYPE_OFFSET if msg.len() >= 24 => (u64_at(msg, 16), 24),
                _ => return Err(ENOSYS),
            };
            // All the message has, read from the client as it is written,
            // into a file sized for it first: a write that faults part-way
            // leaves it so, zeros past what came (procnto's, measured).
            let len = nbytes.min(total.saturating_sub(data_at));
            let end = at.checked_add(len as u64).ok_or(ENOMEM)?;
            if end > file.obj.size() {
                file.obj.set_size(end)?;
            }
            let n = resmgr::write_in(rcvid, msg, total, data_at, nbytes, |off, piece| {
                file.obj.write(at + off as u64, piece)
            })?;
            if u32_at(msg, 8) & XTYPE_MASK == XTYPE_NONE {
                *pos = at + n as u64;
            }
            file.mtime.store(now_secs(), Ordering::Relaxed);
            Ok((n as u32, Vec::new()))
        }
        (IO_LSEEK, o) if msg.len() >= 16 => {
            let (pos, size) = match o {
                Open::File { pos, file, .. } => (pos, file.obj.size()),
                Open::Dir { pos, .. } => (pos, 0),
            };
            let whence = i16::from_le_bytes([msg[4], msg[5]]);
            let offset = u64_at(msg, 8) as i64;
            let base = match whence {
                0 => 0,
                1 => *pos as i64,
                2 => size as i64,
                _ => return Err(EINVAL),
            };
            let new = base
                .checked_add(offset)
                .filter(|&p| p >= 0)
                .ok_or(EOVERFLOW)?;
            *pos = new as u64;
            Ok((0, new.to_le_bytes().to_vec()))
        }
        (
            IO_SPACE,
            Open::File {
                file, pos, ioflag, ..
            },
        ) if msg.len() >= 24 => {
            // An object shm_ctl() gave memory to keeps its size (procnto).
            if file.ctl.load(Ordering::Relaxed) & (SHMCTL_ANON | SHMCTL_PHYS) != 0 {
                return Err(EINVAL);
            }
            let subtype = u16::from_le_bytes([msg[4], msg[5]]);
            let whence = i16::from_le_bytes([msg[6], msg[7]]);
            let size = file.obj.size();
            let base = match whence {
                0 => 0,
                1 => *pos,
                2 => size,
                _ => return Err(EINVAL),
            };
            let start = base
                .checked_add(u64_at(msg, 8))
                .filter(|&s| (s as i64) >= 0);
            let start = start.ok_or(ENOMEM)?;
            let len = u64_at(msg, 16);
            let end = start.checked_add(len).ok_or(EINVAL)?;
            match subtype {
                F_GROWSP | F_ALLOCSP => {
                    // `ftruncate()` growing the object: backed at once, or
                    // ENOMEM with the object unchanged (procnto, measured).
                    if len != 0 && end > size {
                        file.obj.grow(end)?;
                    }
                }
                F_FREESP => {
                    if len != 0 && end < size {
                        return Err(EINVAL);
                    }
                    if *ioflag & IO_FLAG_WR == 0 {
                        return Err(EINVAL);
                    }
                    if start < size {
                        file.obj.set_size(start)?;
                    }
                }
                _ => return Err(EINVAL),
            }
            file.mtime.store(now_secs(), Ordering::Relaxed);
            Ok((0, file.obj.size().to_le_bytes().to_vec()))
        }
        (IO_CHMOD, Open::File { file, .. }) if msg.len() >= 8 => {
            file.mode.store(u32_at(msg, 4) & 0o7777, Ordering::Relaxed);
            Ok((0, Vec::new()))
        }
        // Root's, or the owner's; -1 leaves one as it is.
        (IO_CHOWN, Open::File { file, .. }) if msg.len() >= 12 => {
            let (euid, _) = owner_of(client);
            if euid != 0 && euid != file.uid.load(Ordering::Relaxed) {
                return Err(crate::ker::errno::EPERM);
            }
            let (gid, uid) = (u32_at(msg, 4), u32_at(msg, 8));
            if uid != u32::MAX {
                file.uid.store(uid, Ordering::Relaxed);
            }
            if gid != u32::MAX {
                file.gid.store(gid, Ordering::Relaxed);
            }
            Ok((0, Vec::new()))
        }
        // utime(): the modification time given, or now (`cur_flag`, or
        // UTIME_NOW in the 64-bit form; UTIME_OMIT leaves it).
        (IO_UTIME, Open::File { file, .. }) if msg.len() >= 16 => {
            let t = if u32_at(msg, 4) != 0 {
                now_secs()
            } else {
                u32_at(msg, 12) as u64
            };
            file.mtime.store(t, Ordering::Relaxed);
            Ok((0, Vec::new()))
        }
        (IO_UTIME64, Open::File { file, .. }) if msg.len() >= 40 => {
            const UTIME_NOW: u64 = 0x4000_0001;
            const UTIME_OMIT: u64 = 0x4000_0002;
            let (sec, nsec) = (u64_at(msg, 24), u64_at(msg, 32));
            match nsec {
                UTIME_OMIT => {}
                UTIME_NOW => file.mtime.store(now_secs(), Ordering::Relaxed),
                _ => file.mtime.store(sec, Ordering::Relaxed),
            }
            Ok((0, Vec::new()))
        }
        // A directory takes writes (none of the bytes) and utime(), not
        // truncation, chmod() or ACLs (procnto's).
        (IO_WRITE | IO_WRITE64 | IO_UTIME | IO_UTIME64, Open::Dir { .. }) => Ok((0, Vec::new())),
        (IO_SPACE | IO_CHMOD | IO_ACL, Open::Dir { .. }) => Err(ENOSYS),
        // ACLs (acl_set_fd(), as slogger2 gives its clients access to
        // their buffers): everyone is root, so a set or a check succeeds
        // and leaves nothing to keep (EBADMSG without an ACL after the
        // header); a get gets none (measured).
        (IO_ACL, _) if msg.len() >= 8 => match u32_at(msg, 4) {
            ACL_SET | ACL_CHK if msg.len() <= 16 => Err(EBADMSG),
            ACL_GET | ACL_SET | ACL_CHK => Ok((0, alloc::vec![0u8; 16])),
            _ => Err(ENOSYS),
        },
        _ => Err(ENOSYS),
    }
}

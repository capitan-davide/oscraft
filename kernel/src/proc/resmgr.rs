//! procnto's own resource managers: the servers behind the path names
//! procnto attaches itself (see `pathmgr`): the image filesystem,
//! read-only, at `/`, the directories that only exist in the path space
//! (/dev), and /dev/null and /dev/zero.
//!
//! As in a QNX resource manager, `open()` ends with an `_IO_CONNECT` sent
//! on the client's new connection (its file descriptor), and accepting it
//! binds an open-file context (OCB) to that connection. The I/O messages
//! (`_IO_READ`, `_IO_STAT`, ...) that arrive later on the connection find
//! their OCB by the client's pid and connection ID; `_IO_CLOSE` unbinds it.
//!
//! The image filesystem, as procnto's (fstest): write-opens are EROFS,
//! directories too; links and renames ENOSYS; a plain `read()` of a
//! directory EISDIR (`readdir()` sends `_IO_XTYPE_READDIR`); `lseek()`
//! stays within the file, `pread()` past its end is EINVAL; `statvfs()`
//! gives "ifs". `fcntl(F_GETFL)` reports `O_*` flags and `F_SETFL`
//! changes the settable ones.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::imagefs::{self, Entry, S_IFLNK, S_IFMT};
use super::pathmgr::{self, Connect};
use super::procfs::Node as ProcNode;
use crate::arch::kercall::call;
use crate::ker::errno::{EBADF, EBADMSG, EBUSY, EEXIST, EFAULT, EIO, EINVAL, EISDIR, ENODEV, ENOENT, ENOSYS, ENOTEMPTY, ENOTTY, EROFS};
use crate::ker::number::*;
use crate::kprintln;
use crate::personality::{native::{self, Stat}, qnx};
use crate::sync::IrqSpinLock;

/// Handle of the image filesystem in the path manager's entries.
pub const IMAGEFS_HANDLE: u32 = 1;
/// Handles of /dev/null and /dev/zero.
pub const DEV_NULL_HANDLE: u32 = 2;
pub const DEV_ZERO_HANDLE: u32 = 3;
/// The inode /dev/zero reports (anonymous memory's object, see `memmgr`).
pub const DEV_ZERO_INO: u64 = 3;
/// `/dev/tty`: the session's terminal (see `session`).
pub const DEV_TTY_HANDLE: u32 = 4;
/// The process filesystem at `/proc` (see `procfs`).
pub const PROCFS_HANDLE: u32 = 5;
/// Shared memory objects at `/dev/shmem` (see `shmem`).
pub const SHMEM_HANDLE: u32 = 6;
/// Typed memory (see `tymem`).
pub const TYMEM_HANDLE: u32 = 7;
/// `resmgr_attach()`'s second step, see `pathmgr::attach`.
pub const LINK_HANDLE: u32 = 8;
/// Named semaphores: `/dev/sem` and `_FTYPE_SEM` connects (`namedsem`).
pub const SEM_HANDLE: u32 = 9;
/// The kernel's messages: `/dev/klog` (`msgbuf`).
pub const DEV_KLOG_HANDLE: u32 = 10;
/// The most bytes a read reply is built in at once (`read_out`).
const READ_CHUNK: usize = 64 * 1024;
const S_IFCHR: u32 = 0o020000;
/// Handles from here on are the path space's own directories.
pub const DIR_NODE_HANDLE: u32 = 0x100;
/// Handles of `/proc/mount`'s directories (`pathmgr::mount_dirs`).
pub const MOUNT_NODE_HANDLE: u32 = 0x2000_0000;
/// Handles of `name_attach()` names, as procnto answers connects of other
/// file types to them (`pathmgr::name_nodes`).
pub const NAME_NODE_HANDLE: u32 = 0x4000_0000;

// I/O message types (<sys/iomsg.h>)
const IO_READ: u16 = 0x101;
pub const IO_WRITE: u16 = 0x102;
const IO_CHMOD: u16 = 0x10a;
const IO_CHOWN: u16 = 0x10b;
const IO_SPACE: u16 = 0x110;
const IO_SYNC: u16 = 0x119;
pub const IO_WRITE64: u16 = 0x11f;
const IO_STAT: u16 = 0x104;
const IO_DEVCTL: u16 = 0x106;
const IO_PATHCONF: u16 = 0x108;
const IO_LSEEK: u16 = 0x109;
const IO_CLOSE: u16 = 0x116;
const IO_LOCK: u16 = 0x10f;
const IO_DUP: u16 = 0x115;
const IO_READ64: u16 = 0x11e;
pub const IO_FDINFO: u16 = 0x10e;
const IO_OPENFD: u16 = 0x10d;
// The other `_IO_*` types of `<sys/iomsg.h>`, including its reserved
// "place holder" ones.
const IO_RSVD_CLOSE_OCB: u16 = 0x103;
const IO_NOTIFY: u16 = 0x105;
const IO_RSVD_UNBLOCK: u16 = 0x107;
const IO_UTIME: u16 = 0x10c;
const IO_SHUTDOWN: u16 = 0x111;
const IO_MMAP: u16 = 0x112;
const IO_MSG: u16 = 0x113;
const IO_RSVD_CHECK_MSG: u16 = 0x114;
const IO_RSVD_LOCK_OCB: u16 = 0x117;
const IO_RSVD_UNLOCK_OCB: u16 = 0x118;
const IO_POWER: u16 = 0x11a;
const IO_ACL: u16 = 0x11b;
const IO_RSVD_PAUSE: u16 = 0x11c;
const IO_RSVD_UNPAUSE: u16 = 0x11d;
const IO_NOTIFY64: u16 = 0x120;
const IO_UTIME64: u16 = 0x121;
/// `sizeof(struct _io_fdinfo)`, `sizeof(struct _fdinfo)`.
const FDINFO_MSG_SIZE: usize = 16;
const FDINFO_SIZE: usize = 64;
/// `struct _io_dup`: the original connection ID, in its `info`.
const DUP_COID: usize = 28;
/// `info.pid` in `struct _io_dup`.
const DUP_PID: usize = 12;
/// `DCMD_ALL_GETFLAGS` = `__DIOF(_DCMD_ALL, 1, int)`,
/// `DCMD_ALL_SETFLAGS` = `__DIOT(_DCMD_ALL, 2, int)`,
/// `DCMD_FSYS_STATVFS` = `__DIOF(_DCMD_FSYS, 13, struct __msg_statvfs)`.
const DCMD_ALL_GETFLAGS: u32 = 0x4004_0101;
const DCMD_ALL_SETFLAGS: u32 = 0x8004_0102;
const DCMD_FSYS_STATVFS: u32 = 0x40a8_020d;
/// `sizeof(struct __msg_statvfs)`; `ST_RDONLY`, `ST_TRUSTED`.
const MSG_STATVFS_SIZE: usize = 168;
const ST_RDONLY: u32 = 0x1;
const ST_TRUSTED: u32 = 0x80;
/// The image filesystem's device (`st_dev`, `f_fsid`), as procnto's.
pub(super) const IFS_DEV: u32 = 0x802;
/// `st_dev` of procnto's devices (/dev/null, /dev/zero, /dev/text).
pub const DEVICES_DEV: u32 = 0x1;
/// `st_rdev` of /dev/null, /dev/zero and /dev/text (measured).
const DEV_NULL_RDEV: u32 = 0x400;
const DEV_ZERO_RDEV: u32 = 0x406;
pub const DEV_TEXT_RDEV: u32 = 0x401;
/// `_S_TRUST_EXT`: what procnto's image filesystem marks all its entries.
const S_TRUST_EXT: u32 = 0x2_0000;
const O_DIRECTORY: u32 = 0x2_0000;
// _IO_CONNECT subtypes
const CONNECT_COMBINE: u16 = 0;
const CONNECT_COMBINE_CLOSE: u16 = 1;
const CONNECT_OPEN: u16 = 2;
const CONNECT_UNLINK: u16 = 3;
const CONNECT_RENAME: u16 = 4;
/// `_IO_CONNECT_EXTRA_RENAME`.
const EXTRA_RENAME: u8 = 9;
const CONNECT_MKNOD: u16 = 5;
const CONNECT_READLINK: u16 = 6;
const CONNECT_LINK: u16 = 7;
// _IO_READ xtypes
const XTYPE_MASK: u32 = 0xff;
const XTYPE_NONE: u32 = 0;
const XTYPE_OFFSET: u32 = 5;
const XTYPE_READDIR: u32 = 9;
/// `_IO_FLAG_WR`
const IO_FLAG_WR: u32 = 0x2;
/// `O_ACCMODE` (`<fcntl.h>`)
const O_ACCMODE: u32 = 0x3;
/// `_STAT_FORM_T64_2008`: the `struct stat` of 64-bit programs.
pub const STAT_FORM_T64_2008: u32 = 3;
const SEEK_SET: i16 = 0;
const SEEK_CUR: i16 = 1;
const SEEK_END: i16 = 2;

// ANCHOR: ocb
/// Who sent a message: the client process and its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Client {
    pub pid: i32,
    pub coid: i32,
}

/// An open-file context.
#[derive(Clone)]
enum Ocb {
    /// An image filesystem entry (`path` without leading '/'). For a
    /// directory, `pos` counts the names read so far.
    Image {
        entry: Entry,
        path: String,
        pos: u64,
        ioflag: u32,
    },
    /// A directory in procnto's path space opened with `_PATH_OPENDIR`:
    /// a handle for the `*at()` calls, not for reading.
    PathDir { path: String, ioflag: u32 },
    /// /dev/null or /dev/zero.
    Dev { dev: Dev, ioflag: u32 },
    /// A directory or file of the process filesystem; `pos` as for
    /// `Image`.
    Proc {
        node: super::procfs::Node,
        pos: u64,
        ioflag: u32,
    },
    /// A shared memory object or `/dev/shmem` itself.
    Shm(super::shmem::Open),
    /// A typed memory object.
    Tymem(super::tymem::Open),
    /// A named semaphore.
    Sem(super::namedsem::Sem),
    /// `/dev/sem`; `pos` as for `Image`.
    SemDir { pos: u64, ioflag: u32 },
    /// An open of `/dev/text`, whose server (`devtext`) keeps its open
    /// flags here, so that they follow the descriptor as the others do.
    Text { ioflag: u32 },
    /// A directory of `/proc/mount` (`path` in the path space); `pos` as
    /// for `Image`.
    MountDir { path: String, pos: u64, ioflag: u32 },
}

/// procnto's devices (QNX `pathmgr/devnull.c`, `devzero.c`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Dev {
    /// Reads give end-of-file, writes are discarded.
    Null,
    /// Reads give zeros, writes are discarded.
    Zero,
    /// The kernel's messages, each once, for one reader: root's.
    Klog,
}

/// Who holds `/dev/klog`; another open is EBUSY while it lives.
static KLOG_READER: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(0);

/// `logopen()`, FreeBSD's: root's device, one reader at a time.
fn klog_open(pid: i32) -> Result<(), u32> {
    use core::sync::atomic::Ordering;
    if crate::ker::process::lookup(pid).is_some_and(|p| p.cred.lock().euid != 0) {
        return Err(crate::ker::errno::EACCES);
    }
    let holder = KLOG_READER.load(Ordering::Relaxed);
    if holder != 0 && holder != pid && crate::ker::process::lookup(holder).is_some() {
        return Err(EBUSY);
    }
    KLOG_READER.store(pid, Ordering::Relaxed);
    Ok(())
}

/// One open-file context, shared by every descriptor that refers to it:
/// `dup()`, a forked child's and a spawned child's inherited descriptors
/// all name the same `Arc`, so a position moved or a flag set through
/// one is seen through the others, as POSIX's open file description and
/// procnto's `iofunc_ocb_attach()` (measured, PATHMEM-1: `dup()`'d
/// descriptors at the same offset, `F_SETFL` seen by both, a forked
/// child's writes moving the parent's position).
type OcbRef = alloc::sync::Arc<IrqSpinLock<Ocb>>;

static OCBS: IrqSpinLock<BTreeMap<Client, OcbRef>> = IrqSpinLock::new(BTreeMap::new());
// ANCHOR_END: ocb

/// Bind a fresh open-file context to `client`'s connection.
fn insert_ocb(client: Client, ocb: Ocb) -> Option<OcbRef> {
    OCBS.lock().insert(client, alloc::sync::Arc::new(IrqSpinLock::new(ocb)))
}

/// procnto never binds an open-file context over one the connection
/// still has (left by a `ConnectDetach()` without `close()`): an open, a
/// `shm_open()`, a `dup()` on that descriptor number is EBUSY, before
/// anything is made, for as long as the process lives; a `stat()`'s or
/// `access()`'s open-and-close leaves it be (measured, review PATHMEM-16).
pub fn unbound(client: Client) -> Result<(), u32> {
    if OCBS.lock().contains_key(&client) { Err(EBUSY) } else { Ok(()) }
}

/// The open-file context of `client`'s connection, to lock.
fn ocb_of(client: Client) -> Option<OcbRef> {
    OCBS.lock().get(&client).cloned()
}

pub fn reply(rcvid: u64, status: u32, data: &[u8]) {
    let _ = call(
        __KER_MSG_REPLYV,
        [
            rcvid,
            status as u64,
            data.as_ptr() as u64,
            (-(data.len() as i64)) as u64,
            0,
            0,
        ],
    );
}

pub fn error(rcvid: u64, errno: u32) {
    let _ = call(__KER_MSG_ERROR, [rcvid, errno as u64, 0, 0, 0, 0]);
}

/// The data of a write: `nbytes` from offset `at` of the client's message,
/// `total` bytes long, of which `msg` arrived with it. procnto takes a
/// write whole, however long (measured, issue #285: 64 MiB to a
/// `/dev/shmem` file in one `write()`), so the rest is read from the
/// client (MsgRead) piece by piece, through one `READ_CHUNK` buffer, and
/// handed to `sink(offset, piece)` as it comes. Returns how many bytes
/// there were (all `nbytes` the message has). A client buffer that faults
/// part-way is EFAULT, after every byte before the fault went to `sink`
/// (procnto's, measured).
pub fn write_in(
    rcvid: u64,
    msg: &[u8],
    total: usize,
    at: usize,
    nbytes: usize,
    mut sink: impl FnMut(usize, &[u8]) -> Result<(), u32>,
) -> Result<usize, u32> {
    let len = nbytes.min(total.saturating_sub(at));
    let here = msg.len().saturating_sub(at).min(len);
    if here > 0 {
        sink(0, &msg[at..at + here])?;
    }
    let mut done = here;
    let mut buf = Vec::new();
    let read = |buf: &mut [u8], from: usize| {
        call(
            __KER_MSG_READV,
            [rcvid, buf.as_mut_ptr() as u64, (-(buf.len() as i64)) as u64, from as u64, 0, 0],
        )
        .map(|n| n as usize)
    };
    while done < len {
        buf.resize((len - done).min(READ_CHUNK), 0);
        let got = match read(&mut buf, at + done) {
            Ok(got) => got,
            // The client's buffer ends in this piece: as much of it as
            // there is (found by halves) goes in before the error.
            Err(EFAULT) => {
                let (mut good, mut bad) = (0, buf.len());
                while bad - good > 1 {
                    let mid = (good + bad) / 2;
                    if read(&mut buf[..mid], at + done).is_ok() {
                        good = mid;
                    } else {
                        bad = mid;
                    }
                }
                if good > 0 && read(&mut buf[..good], at + done).is_ok() {
                    sink(done, &buf[..good])?;
                }
                return Err(EFAULT);
            }
            Err(e) => return Err(e),
        };
        if got == 0 {
            break;
        }
        sink(done, &buf[..got])?;
        done += got;
    }
    Ok(done)
}

/// The data of a read of up to `len` bytes, for the reply. procnto copies
/// a whole request, however long (measured, review PATHMEM-14: 1 MiB from
/// a `/dev/shmem` file, `/dev/zero` or `/proc/<pid>/as` in one read), so
/// a longer one is written into the client's buffer piece by piece
/// (MsgWrite), through one `READ_CHUNK` buffer, and the reply carries no
/// data. `fill(offset, piece)` fills a piece and says how many bytes it
/// has; a short one ends the read. A short read (`read()`'s returned
/// count) and the data to reply with come back; a fault in the client's
/// buffer is EFAULT, all of the read failing (as procnto's).
pub fn read_out(
    rcvid: u64,
    len: usize,
    mut fill: impl FnMut(usize, &mut [u8]) -> Result<usize, u32>,
) -> Result<(usize, Vec<u8>), u32> {
    let mut buf = alloc::vec![0u8; len.min(READ_CHUNK)];
    if len <= READ_CHUNK {
        let n = fill(0, &mut buf)?;
        buf.truncate(n);
        return Ok((n, buf));
    }
    let mut done = 0;
    while done < len {
        let piece = &mut buf[..(len - done).min(READ_CHUNK)];
        let n = fill(done, piece)?;
        let wrote = call(
            __KER_MSG_WRITEV,
            [rcvid, piece.as_ptr() as u64, (-(n as i64)) as u64, done as u64, 0, 0],
        )?;
        done += wrote as usize;
        if n < piece.len() || (wrote as usize) < n {
            break;
        }
    }
    Ok((done, Vec::new()))
}

/// Where a symbolic link at `link` (image path) leads, with `rest` (the
/// path after the link, starting with '/' or empty) appended. Absolute.
fn link_target(link: &str, target: &[u8], rest: &str) -> String {
    let target = core::str::from_utf8(target).unwrap_or("");
    if target.starts_with('/') {
        format!("{target}{rest}")
    } else {
        match link.rfind('/') {
            Some(i) => format!("/{}/{target}{rest}", &link[..i]),
            None => format!("/{target}{rest}"),
        }
    }
}

/// `_IO_CONNECT` for a directory of the path space (`pathmgr::dir_nodes`).
/// A directory of `/proc/mount` (see `pathmgr::resolve_mount`): read
/// only, its names listed.
fn connect_mount_node(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    let path = pathmgr::node_path(c.handle - MOUNT_NODE_HANDLE)
        .filter(|p| pathmgr::mount_dirs().contains(p))
        .ok_or(ENOENT)?;
    let path = &path;
    if !c.path.trim_matches('/').is_empty() {
        return Err(ENOENT);
    }
    if c.ioflag & IO_FLAG_WR != 0 {
        return Err(EROFS);
    }
    answer_open(rcvid, client, c, Ocb::MountDir { path: path.clone(), pos: 0, ioflag: c.ioflag })
}

fn connect_dir_node(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    // The node the handle names, if it still exists (the ID is for life,
    // the directory is there while a server is attached below it).
    let path = pathmgr::node_path(c.handle - DIR_NODE_HANDLE)
        .filter(|p| pathmgr::dir_nodes().contains(p))
        .ok_or(ENOENT)?;
    let path = &path;
    if !c.path.trim_matches('/').is_empty() {
        return Err(ENOENT);
    }
    // As procnto's pathmgr_domknod(): the directory exists, and has names
    // below it. One the image filesystem has too can't be removed (EROFS,
    // as measured on procnto).
    match c.subtype {
        CONNECT_MKNOD => return Err(EEXIST),
        CONNECT_UNLINK if imagefs::get().is_some_and(|fs| fs.entry(path).is_some()) => return Err(EROFS),
        CONNECT_UNLINK => return Err(ENOTEMPTY),
        CONNECT_READLINK => return Err(EINVAL),
        _ => {}
    }
    let entry = Entry {
        ino: pathmgr::node_ino(path) as u32,
        mode: imagefs::S_IFDIR | 0o555,
        uid: 0,
        gid: 0,
        mtime: 0,
        data: &[],
    };
    open(rcvid, client, c, entry, path)
}

/// A `name_attach()` name reached by a connect of another file type than
/// `_FTYPE_NAME` (a `stat()`, an `open()`): procnto's node for it, an
/// `S_IFNAM` entry that opens (for reading or writing), exists (EEXIST for
/// mkdir()), isn't a link (EINVAL) and has nothing to unlink (ENOSYS), as
/// procnto's.
fn connect_name_node(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    const S_IFNAM: u32 = 0o050000;
    let path = pathmgr::node_path(c.handle - NAME_NODE_HANDLE)
        .filter(|p| pathmgr::name_nodes().contains(p))
        .ok_or(ENOENT)?;
    let path = &path;
    if !c.path.trim_matches('/').is_empty() {
        return Err(ENOENT);
    }
    match c.subtype {
        CONNECT_OPEN | CONNECT_COMBINE | CONNECT_COMBINE_CLOSE => {}
        CONNECT_MKNOD => return Err(EEXIST),
        CONNECT_READLINK => return Err(EINVAL),
        _ => return Err(ENOSYS),
    }
    let entry = Entry {
        ino: pathmgr::node_ino(path) as u32,
        mode: S_IFNAM | 0o666,
        uid: 0,
        gid: 0,
        mtime: 0,
        data: &[],
    };
    let ocb = Ocb::Image {
        entry,
        path: path.clone(),
        pos: 0,
        ioflag: c.ioflag,
    };
    answer_open(rcvid, client, c, ocb)
}

/// `_IO_CONNECT` for procnto's resource managers: the image filesystem
/// (`c.path` relative to `/`) and the path space's own directories.
pub fn connect(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    if c.handle >= NAME_NODE_HANDLE {
        return connect_name_node(rcvid, client, c);
    }
    if c.handle >= MOUNT_NODE_HANDLE {
        return connect_mount_node(rcvid, client, c);
    }
    if c.handle >= DIR_NODE_HANDLE {
        return connect_dir_node(rcvid, client, c);
    }
    if c.handle == PROCFS_HANDLE {
        // /proc/boot is one of procfs's names on procnto (the image
        // filesystem's directory): nothing to make or remove there.
        if matches!(c.subtype, CONNECT_MKNOD | CONNECT_UNLINK) && c.path.trim_end_matches(['/', '.']) == "boot" {
            return Err(EROFS);
        }
        // A path's stat() is an open, the stat and a close in one message.
        let snapshot = c.subtype != CONNECT_COMBINE_CLOSE;
        let node = super::procfs::open(client.pid, c.path, c.ioflag & IO_FLAG_WR != 0, snapshot)?;
        let ioflag = c.ioflag;
        return answer_open(
            rcvid,
            client,
            c,
            Ocb::Proc {
                node,
                pos: 0,
                ioflag,
            },
        );
    }
    if c.handle == TYMEM_HANDLE {
        let open = super::tymem::open(c)?;
        return answer_open(rcvid, client, c, Ocb::Tymem(open));
    }
    if c.handle == SHMEM_HANDLE {
        // No links, symbolic or hard (procnto's: ENOSYS).
        if c.subtype == CONNECT_LINK {
            return Err(crate::ker::errno::ENOSYS);
        }
        // rename(): the old name, relative to /dev/shmem too, in the extra.
        if c.subtype == CONNECT_RENAME && c.extra_type == EXTRA_RENAME {
            let old = c.combined.split(|&b| b == 0).next().unwrap_or(&[]);
            let old = core::str::from_utf8(old).map_err(|_| EINVAL)?;
            super::shmem::rename(old, c.path)?;
            reply(rcvid, 0, &[]);
            return Ok(());
        }
        if c.subtype == CONNECT_UNLINK {
            super::shmem::unlink(c.path)?;
            reply(rcvid, 0, &[]);
            return Ok(());
        }
        // No mknod(): procnto's /dev/shmem has no handler for it
        // (`shmem_connect_funcs`, read with REA).
        if c.subtype == CONNECT_MKNOD {
            return Err(crate::ker::errno::ENOSYS);
        }
        let umask = crate::ker::process::lookup(client.pid)
            .map_or(0, |p| p.umask.load(core::sync::atomic::Ordering::Relaxed));
        // An open that may create the file: refused (`unbound`) before
        // it does (measured).
        if c.ioflag & super::shmem::O_CREAT != 0 && matches!(c.subtype, CONNECT_OPEN | CONNECT_COMBINE) {
            unbound(client)?;
        }
        let open = super::shmem::open(c, umask, client.pid)?;
        return answer_open(rcvid, client, c, Ocb::Shm(open));
    }
    if c.handle == SEM_HANDLE {
        return sem_connect(rcvid, client, c);
    }
    if c.handle == DEV_TTY_HANDLE {
        let to = super::session::tty_name(client.pid).ok_or(crate::ker::errno::ENXIO)?;
        return pathmgr::link_reply(rcvid, c, &[], &to);
    }
    let dev = match c.handle {
        DEV_NULL_HANDLE => Some(Dev::Null),
        DEV_ZERO_HANDLE => Some(Dev::Zero),
        DEV_KLOG_HANDLE => Some(Dev::Klog),
        _ => None,
    };
    if let Some(dev) = dev {
        // A path below the device (only `/proc/mount` sends one): ENOTDIR,
        // as procnto's (measured).
        if !c.path.trim_matches('/').is_empty() {
            return Err(crate::ker::errno::ENOTDIR);
        }
        // Nothing to unlink or make here: the next server answers (procnto).
        match c.subtype {
            CONNECT_OPEN | CONNECT_COMBINE | CONNECT_COMBINE_CLOSE => {}
            _ => return Err(ENOSYS),
        }
        if dev == Dev::Klog {
            klog_open(client.pid)?;
        }
        let ioflag = c.ioflag;
        return answer_open(rcvid, client, c, Ocb::Dev { dev, ioflag });
    }
    if c.handle != IMAGEFS_HANDLE {
        return Err(ENOENT);
    }
    // The image filesystem makes no links and renames nothing (procnto).
    if c.subtype == CONNECT_RENAME || c.subtype == CONNECT_LINK {
        return Err(crate::ker::errno::ENOSYS);
    }
    let fs = imagefs::get().ok_or(ENOENT)?;
    let path = c.path.trim_matches('/');
    // A symbolic link on the way is resolved by the client.
    for (i, _) in path.match_indices('/') {
        if let Some(e) = fs.entry(&path[..i])
            && e.mode & S_IFMT == S_IFLNK
        {
            let to = link_target(&path[..i], e.data, &path[i..]);
            return pathmgr::link_reply(rcvid, c, &[], &to);
        }
    }
    // As procnto's image_mknod(): a name that exists can't be made again
    // or removed; one that doesn't is for the next server to make (ENOSYS),
    // and isn't there to remove.
    let entry = fs.entry(path);
    match (c.subtype, &entry) {
        (CONNECT_MKNOD, Some(_)) => return Err(EEXIST),
        (CONNECT_MKNOD, None) => return Err(ENOSYS),
        (CONNECT_UNLINK, Some(_)) => return Err(EROFS),
        _ => {}
    }
    let entry = entry.ok_or(ENOENT)?;
    if entry.mode & S_IFMT == S_IFLNK {
        if c.subtype == CONNECT_READLINK {
            let target = core::str::from_utf8(entry.data).map_err(|_| EINVAL)?;
            return pathmgr::readlink_reply(rcvid, c, target);
        }
        // Unless the link itself is asked for (lstat()), follow it; a
        // trailing '/' or "/." (the eflag of `pathmgr::resolve`) names its
        // target.
        if c.mode & S_IFMT != S_IFLNK || c.eflag & pathmgr::EFLAG_DIR != 0 {
            return pathmgr::link_reply(rcvid, c, &[], &link_target(path, entry.data, ""));
        }
    } else if c.subtype == CONNECT_READLINK {
        return Err(EINVAL);
    }
    open(rcvid, client, c, entry, path)
}

/// A connect to the named semaphores: `sem_open()` and `sem_unlink()`
/// (`_FTYPE_SEM`), or a name in `/dev/sem` or that directory itself.
fn sem_connect(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    use super::namedsem;
    // Nothing is made in /dev/sem but by sem_open() (procnto: ENOSYS).
    if c.subtype == CONNECT_MKNOD {
        return Err(ENOSYS);
    }
    if c.subtype == CONNECT_UNLINK {
        namedsem::unlink(c.path)?;
        reply(rcvid, 0, &[]);
        return Ok(());
    }
    if c.file_type == namedsem::FTYPE_SEM {
        let p = crate::ker::process::lookup(client.pid).ok_or(ENOENT)?;
        let umask = p.umask.load(core::sync::atomic::Ordering::Relaxed);
        let cred = p.cred.lock().clone();
        let sem = namedsem::open(c, umask, &cred)?;
        return answer_open(rcvid, client, c, Ocb::Sem(sem));
    }
    if c.path.trim_matches('/').is_empty() {
        let ioflag = c.ioflag;
        return answer_open(rcvid, client, c, Ocb::SemDir { pos: 0, ioflag });
    }
    let sem = namedsem::lookup(c.path).ok_or(ENOENT)?;
    answer_open(rcvid, client, c, Ocb::Sem(sem))
}

/// The named semaphore a client's connection has open (for the kernel's
/// `SyncSem*()` on a `sem_open()` semaphore, and `SIGEV_SEM`).
pub fn named_sem(client: Client) -> Option<alloc::sync::Arc<crate::ker::sync::NamedSem>> {
    let ocb = ocb_of(client)?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::Sem(s) => Some(s.sem.clone()),
        _ => None,
    }
}

/// Bind an open-file context for `entry` (at `path`) to the client's
/// connection and answer the connect (or the I/O message combined with it).
fn open(rcvid: u64, client: Client, c: &Connect, entry: Entry, path: &str) -> Result<(), u32> {
    match c.subtype {
        CONNECT_OPEN | CONNECT_COMBINE | CONNECT_COMBINE_CLOSE => {}
        _ => return Err(EROFS),
    }
    // A write-open of anything here, directories too: EROFS (procnto).
    if c.ioflag & IO_FLAG_WR != 0 {
        return Err(EROFS);
    }
    let ocb = Ocb::Image {
        entry,
        path: path.into(),
        pos: 0,
        ioflag: c.ioflag,
    };
    answer_open(rcvid, client, c, ocb)
}

/// Bind `ocb` to the client's connection and answer the connect (or the
/// I/O message combined with it).
fn answer_open(rcvid: u64, client: Client, c: &Connect, ocb: Ocb) -> Result<(), u32> {
    match c.subtype {
        CONNECT_OPEN | CONNECT_COMBINE | CONNECT_COMBINE_CLOSE => {}
        _ => return Err(EROFS),
    }
    // Once the name is found (a missing one is ENOENT first): not over a
    // context the connection still has (`unbound`), which comes back
    // after a combined close.
    if c.subtype != CONNECT_COMBINE_CLOSE {
        unbound(client)?;
    }
    let prior = insert_ocb(client, ocb);
    let close = || {
        let gone = {
            let mut ocbs = OCBS.lock();
            match prior.clone() {
                Some(p) => ocbs.insert(client, p),
                None => ocbs.remove(&client),
            }
        };
        // Let go of it after the table (`release`).
        release(gone);
    };
    if c.subtype == CONNECT_OPEN {
        reply(rcvid, 0, &[]);
        return Ok(());
    }
    // The combined I/O message (e.g. the `_IO_STAT` of open() + fstat())
    // gives the reply; with _IO_CONNECT_COMBINE_CLOSE the file is closed
    // again afterwards.
    if c.combined.len() < 4 {
        // Nothing combined (getcwd() only resolves the path).
        if c.subtype == CONNECT_COMBINE_CLOSE {
            close();
        }
        reply(rcvid, 0, &[]);
        return Ok(());
    }
    let ty = u16::from_le_bytes([c.combined[0], c.combined[1]]);
    io(rcvid, client, ty, c.combined);
    if c.subtype == CONNECT_COMBINE_CLOSE {
        close();
    }
    Ok(())
}

/// An open-file context made without a connect (a spawned child's file
/// action), to be bound to a connection with `bind`.
#[derive(Clone)]
pub struct Opened(Ocb);

/// Open `path` for process `pid` with `open()` flags `oflags`, if procnto
/// serves it itself: a file or directory of the image filesystem (read
/// only), /dev/null or /dev/zero. ENOSYS for other servers' paths.
pub fn open_local(pid: i32, path: &str, oflags: u32, mode: u32) -> Result<Opened, u32> {
    // `_IO_FLAG_RD`/`_IO_FLAG_WR` are the access mode plus one.
    let ioflag = qnx::ioflag_in((oflags & !O_ACCMODE) | ((oflags & O_ACCMODE) + 1));
    let path = pathmgr::absolute(pid, path);
    let dev = match path.as_str() {
        "dev/null" => Some(Dev::Null),
        "dev/zero" => Some(Dev::Zero),
        _ => None,
    };
    if let Some(dev) = dev {
        return Ok(Opened(Ocb::Dev { dev, ioflag }));
    }
    if let Some((entry, path)) = imagefs::get().and_then(|fs| fs.follow(&path)) {
        if ioflag & IO_FLAG_WR != 0 {
            return Err(EROFS);
        }
        return Ok(Opened(Ocb::Image {
            entry,
            path,
            pos: 0,
            ioflag,
        }));
    }
    // A file of procnto's /dev/shmem, through the image's links (/tmp).
    if let Some(fs) = imagefs::get()
        && let Some(name) = fs.resolve(&path).strip_prefix("dev/shmem/").map(String::from)
    {
        let umask = crate::ker::process::lookup(pid).map_or(0, |p| p.umask.load(core::sync::atomic::Ordering::Relaxed));
        let c = pathmgr::Connect {
            subtype: 2, // _IO_CONNECT_OPEN
            file_type: 0, // _FTYPE_ANY
            reply_max: 0,
            handle: 0,
            ioflag,
            mode: (mode & 0o7777) | 0o100000,
            eflag: 0,
            extra_type: 0,
            dirfd: 0,
            path: &name,
            combined: &[],
        };
        return super::shmem::open(&c, umask, pid).map(|o| Opened(Ocb::Shm(o)));
    }
    match pathmgr::lookup(&path) {
        Some(_) => Err(ENOSYS),
        None => Err(ENOENT),
    }
}

/// The path of a directory a client opened with `O_DIRECTORY`
/// (`_PATH_OPENDIR`), for `POSIX_SPAWN_SETCWD`.
pub fn opendir_path(pid: i32, fd: i32) -> Option<String> {
    let ocb = ocb_of(Client { pid, coid: fd })?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::PathDir { path, .. } => Some(path.clone()),
        _ => None,
    }
}

/// `shm_open_handle()`: open the handle in `msg` on the client's new
/// connection.
pub fn shm_handle_open(rcvid: u64, client: Client, msg: &[u8]) {
    match super::shmhandle::open(client.pid, msg) {
        Ok(open) => {
            insert_ocb(client, Ocb::Shm(open));
            reply(rcvid, 0, &msg[..super::shmhandle::MSG_SIZE]);
        }
        Err(e) => error(rcvid, e),
    }
}

/// The typed memory object open on a client connection (for `mmap()`).
pub fn tymem_object(client: Client) -> Option<super::tymem::Open> {
    let ocb = ocb_of(client)?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::Tymem(o) => Some(o.clone()),
        _ => None,
    }
}

/// `mmap()`'s shared-memory object, with the descriptor's open flags:
/// the access mode the mapping must respect.
pub fn shm_file_ioflag(client: Client) -> Option<(alloc::sync::Arc<super::shmem::ShmFile>, u32)> {
    let ocb = ocb_of(client)?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::Shm(o) => Some((super::shmem::file(o)?, super::shmem::ioflag(o))),
        _ => None,
    }
}

/// `/dev/text`'s open flags (`_IO_FLAG_*` and `O_*`) on a client's
/// connection; None for one it wasn't opened on (the descriptors procnto
/// gives a new process).
pub fn text_ioflag(client: Client) -> Option<u32> {
    let ocb = ocb_of(client)?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::Text { ioflag } => Some(*ioflag),
        _ => None,
    }
}

/// An open of `/dev/text` on a client's connection, with `ioflag`
/// (None: forget it, `_IO_CLOSE`).
pub fn text_bind(client: Client, ioflag: Option<u32>) {
    let mut ocbs = OCBS.lock();
    match ioflag {
        Some(ioflag) => {
            ocbs.insert(client, alloc::sync::Arc::new(IrqSpinLock::new(Ocb::Text { ioflag })));
        }
        None => {
            if ocbs.get(&client).is_some_and(|o| matches!(&*o.lock(), Ocb::Text { .. })) {
                ocbs.remove(&client);
            }
        }
    }
}

/// The open flags as `fcntl(F_GETFL)` reports them: the access mode back
/// from `_IO_FLAG_*`, without the flags only the open used (`O_CREAT`,
/// `O_TRUNC`, `O_EXCL`, `O_NOCTTY`): procnto's files never report those
/// (measured, issue #291).
pub fn getfl(ioflag: u32) -> u32 {
    const OPEN_ONLY: u32 = 0x200 | 0x400 | 0x800 | 0x8000;
    let ioflag = ioflag & !OPEN_ONLY;
    (ioflag & !O_ACCMODE) | (ioflag & O_ACCMODE).saturating_sub(1)
}

/// `getfl()` in `client`'s flags.
pub fn getfl_for(client: i32, ioflag: u32) -> u32 {
    if native(client) { getfl(ioflag) } else { qnx::status_out(getfl(ioflag)) }
}

/// Bind an open-file context to a client's connection.
pub fn bind(client: Client, opened: Opened) {
    insert_ocb(client, opened.0);
}

/// The contents of the file open on a client connection and its path
/// (for introspection), for `mmap()`.
pub fn file_contents(client: Client) -> Result<(&'static [u8], &'static str), u32> {
    // The binding must be for a connection to process 1's channel (not a
    // stale one from a connection ID reused for another server).
    let conn = crate::ker::process::lookup(client.pid)
        .and_then(|p| p.connection(client.coid))
        .ok_or(EBADF)?;
    let ours = crate::ker::process::KERNEL.channel(crate::ker::process::SYSMGR_CHID);
    if !ours.is_some_and(|ch| alloc::sync::Arc::ptr_eq(&ch, &conn.channel)) {
        return Err(ENODEV);
    }
    let ocb = ocb_of(client);
    let ocb = ocb.as_ref().map(|o| o.lock());
    match ocb.as_deref() {
        Some(Ocb::Image { entry, path, .. }) if !entry.is_dir() => {
            Ok((entry.data, crate::vm::map::object_name(path)))
        }
        // /dev/zero maps as zeros (an empty file, then zero fill).
        Some(Ocb::Dev { dev: Dev::Zero, .. }) => Ok((&[], "dev/zero")),
        Some(_) => Err(ENODEV),
        None => Err(EBADF),
    }
}

/// The `struct stat` (`_STAT_FORM_T64_2008`) of an entry.
pub fn stat(e: &Entry) -> StatReply {
    let size = e.data.len() as u64;
    let t = [i64::from(e.mtime as u32), 0];
    StatReply {
        st: Stat {
            st_ino: e.ino as u64,
            st_size: size as i64,
            st_uid: e.uid,
            st_gid: e.gid,
            st_mode: e.mode as u16,
            st_nlink: 1,
            st_blksize: 4096,
            st_blocks: size.div_ceil(512) as i64,
            st_mtim: t,
            st_atim: t,
            st_ctim: t,
            st_birthtim: t,
            ..Stat::default()
        },
        extra: qnx::STAT_EXTRA,
    }
}

/// Whether `client` takes FreeBSD's structs: a FreeBSD process, not QNX's
/// nor process 1 itself (`proc::client`).
pub fn native(client: i32) -> bool {
    client != crate::ker::process::KERNEL_PID
        && crate::ker::process::lookup(client)
            .is_some_and(|p| p.personality() == crate::ker::process::Personality::Native)
}

// ANCHOR: stat_encode
/// A `struct stat` to answer with: FreeBSD's, and what QNX's adds.
pub struct StatReply {
    pub st: Stat,
    pub extra: qnx::StatExtra,
}

impl StatReply {
    /// The reply's status and data for `client`: FreeBSD's struct, or
    /// QNX's for a QNX client and for process 1 itself (`proc::client`).
    pub fn encode(&self, client: i32) -> (u32, Vec<u8>) {
        if native(client) {
            (native::STAT_FORM_FREEBSD, native::stat_bytes(&self.st).to_vec())
        } else {
            (STAT_FORM_T64_2008, qnx::stat_out(&self.st, self.extra).to_vec())
        }
    }
}
// ANCHOR_END: stat_encode

/// The `struct stat` of one of procnto's devices: `st_dev` 1, its
/// `st_rdev`, no preferred block size and no blocks (measured).
pub fn dev_stat(ino: u32, mode: u32, size: u64, rdev: u32) -> StatReply {
    let mut s = stat(&Entry {
        ino,
        mode,
        uid: 0,
        gid: 0,
        mtime: 0,
        data: &[],
    });
    s.st.st_size = size as i64;
    s.st.st_dev = u64::from(DEVICES_DEV);
    s.st.st_rdev = u64::from(rdev);
    s.st.st_blksize = 0;
    s
}

/// The `struct stat` of image filesystem entry `e` at `path`, as procnto's:
/// every entry is trusted (`_S_TRUST_EXT`) and on device 0x802; a
/// directory has two links and its number of entries, rounded up to a
/// multiple of 10, as its size; a symbolic link has no blocks (measured).
fn image_stat(e: &Entry, path: &str) -> StatReply {
    let mut s = stat(e);
    let size = image_size(e, path);
    s.st.st_size = size as i64;
    s.st.st_dev = u64::from(IFS_DEV);
    s.extra.mode_bits = S_TRUST_EXT;
    if e.is_dir() {
        s.st.st_nlink = 2;
    }
    s.st.st_blocks = if e.mode & S_IFMT == S_IFLNK { 0 } else { size.div_ceil(512) } as i64;
    s
}

/// The size procnto gives image filesystem entry `e` at `path`: a
/// directory's is its number of entries rounded up to a multiple of 10.
fn image_size(e: &Entry, path: &str) -> u64 {
    if e.is_dir() {
        imagefs::get().map_or(0, |fs| fs.children(path).len().div_ceil(10) * 10) as u64
    } else {
        e.data.len() as u64
    }
}

/// The reply to `_IO_FDINFO` (`iofunc_fdinfo_default()`): status the
/// path's size with its NUL (what `iofdinfo()` returns), data `struct _io_fdinfo_reply` (two zero words, a
/// `struct _fdinfo`) and as much of the path as `msg`'s `path_len`
/// allows, NUL-terminated.
pub fn fdinfo(
    msg: &[u8],
    mode: u32,
    ioflag: u32,
    offset: u64,
    size: u64,
    path: &str,
) -> (u32, Vec<u8>) {
    let path_len = if msg.len() >= FDINFO_MSG_SIZE {
        u32_at(msg, 8) as usize
    } else {
        0
    };
    let mut r = alloc::vec![0u8; 8 + FDINFO_SIZE];
    r[8..12].copy_from_slice(&mode.to_le_bytes());
    r[12..16].copy_from_slice(&ioflag.to_le_bytes());
    r[16..24].copy_from_slice(&offset.to_le_bytes());
    r[24..32].copy_from_slice(&size.to_le_bytes());
    // count, rcount, wcount: this open.
    r[38..40].copy_from_slice(&1u16.to_le_bytes());
    r[40..42].copy_from_slice(&((ioflag & 1) as u16).to_le_bytes());
    r[42..44].copy_from_slice(&((ioflag >> 1 & 1) as u16).to_le_bytes());
    if path_len > 0 {
        let n = path.len().min(path_len - 1);
        r.extend_from_slice(&path.as_bytes()[..n]);
        r.push(0);
    }
    (path.len() as u32 + 1, r)
}

/// The offset of an `_IO_READ`/`_IO_WRITE` with `_IO_XTYPE_OFFSET` (pread()
/// or pwrite(): `struct _xtype_offset` after the header), None for a plain
/// one.
fn xtype_offset(msg: &[u8]) -> Result<Option<u64>, u32> {
    match u32_at(msg, 8) & XTYPE_MASK {
        XTYPE_NONE => Ok(None),
        XTYPE_OFFSET if msg.len() >= 24 => {
            Ok(Some(u64::from_le_bytes(msg[16..24].try_into().unwrap())))
        }
        _ => Err(ENOSYS),
    }
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// Give a forked child its parent's open-file contexts, shared (its
/// connections have the same IDs). QNX's loader sends `_IO_DUP` to each
/// server from the child instead; procnto's own servers can just share.
pub fn fork(parent: i32, child: i32) {
    let mut ocbs = OCBS.lock();
    let copies: Vec<(Client, OcbRef)> = ocbs
        .iter()
        .filter(|(c, _)| c.pid == parent)
        .map(|(c, o)| (Client { pid: child, ..*c }, o.clone()))
        .collect();
    ocbs.extend(copies);
}

/// Give connection `to` the open-file context of `from`, shared (a file
/// descriptor inherited by a spawned process).
pub fn inherit(from: Client, to: Client) {
    let mut ocbs = OCBS.lock();
    // `.cloned()` clones the `Arc`: the same context, not a copy of it.
    if let Some(ocb) = ocbs.get(&from).cloned() {
        ocbs.insert(to, ocb);
    }
}

/// Forget the open-file contexts of a process that has ended.
pub fn process_exited(pid: i32) {
    let gone: Vec<OcbRef> = {
        let mut ocbs = OCBS.lock();
        let keys: Vec<Client> = ocbs.keys().filter(|c| c.pid == pid).copied().collect();
        keys.iter().filter_map(|c| ocbs.remove(c)).collect()
    };
    release(gone);
    super::locks::process_exited(pid);
}

/// Drop open-file contexts taken out of the table, after its lock: the
/// last reference to a shared memory object frees all its memory, which
/// mustn't happen with interrupts masked (review PATHMEM-4: 512 MiB took
/// 0.76 s).
fn release<T>(gone: T) {
    drop(gone);
}

/// Forget the open-file context of a connection that was closed without
/// `_IO_CLOSE` (close-on-exec).
pub fn detached(client: Client) {
    let gone = OCBS.lock().remove(&client);
    if let Some(ocb) = gone
        && let Ocb::Shm(o) = &*ocb.lock()
        && let Some((key, ..)) = super::shmem::lock_target(o)
    {
        super::locks::release(key, client.pid);
    }
}

/// The path of directory descriptor `fd` of process `pid` (an open
/// image directory or a `_PATH_OPENDIR` handle), for the `*at()` calls.
pub fn path_dir(pid: i32, fd: i32) -> Option<String> {
    let ocb = ocb_of(Client { pid, coid: fd })?;
    let ocb = ocb.lock();
    match &*ocb {
        Ocb::PathDir { path, .. } => Some(path.clone()),
        Ocb::Image { entry, path, .. } if entry.is_dir() => Some(path.clone()),
        Ocb::Image { .. }
        | Ocb::Dev { .. }
        | Ocb::Proc { .. }
        | Ocb::Shm(_)
        | Ocb::Tymem(_)
        | Ocb::Sem(_)
        | Ocb::SemDir { .. }
        | Ocb::Text { .. }
        | Ocb::MountDir { .. } => None,
    }
}

/// A directory handle (`_PATH_OPENDIR`) on a client's connection.
pub fn bind_path_dir(client: Client, path: String, ioflag: u32) {
    insert_ocb(client, Ocb::PathDir { path, ioflag });
}

/// The names in image directory `path` (and those attached below it),
/// with their inodes; as procnto's, no "." and "..".
fn image_names(path: &str) -> Vec<(u64, String)> {
    let mut names: Vec<(u64, String)> = Vec::new();
    if let Some(fs) = imagefs::get() {
        names.extend(
            fs.children(path)
                .into_iter()
                .map(|(i, n)| (i as u64, n.into())),
        );
    }
    // An attached name's inode is its path-space node's, the one its
    // `stat()` reports when the path manager answers it.
    for m in pathmgr::mount_names(path) {
        if !names.iter().any(|(_, n)| *n == m) {
            let full = if path.is_empty() { m.clone() } else { alloc::format!("{path}/{m}") };
            names.push((pathmgr::node_ino(&full), m));
        }
    }
    names
}

/// `struct dirent` records for `names` from index `from` on, as many as
/// fit in `room` bytes, as `client` reads them. Returns them and the
/// number of names.
pub(super) fn dirents(client: i32, names: &[(u64, String)], from: usize, room: usize) -> (Vec<u8>, usize) {
    let native = native(client);
    let mut out = Vec::new();
    let mut count = 0;
    for (i, (ino, name)) in names.iter().enumerate().skip(from) {
        let rec = if native { native::dirent(*ino, i as u64, name) } else { qnx::dirent(*ino, i as u64, name) };
        if out.len() + rec.len() > room {
            break;
        }
        out.extend_from_slice(&rec);
        count += 1;
    }
    (out, count)
}

/// What a request is answered with: a reply status and data.
enum Answer {
    Static(u32, &'static [u8]),
    Owned(u32, Vec<u8>),
}

fn stat_answer(s: &StatReply, client: Client) -> Answer {
    let (status, data) = s.encode(client.pid);
    Answer::Owned(status, data)
}

/// The directory entry an open-file context describes, for `stat()`.
fn ocb_entry(ocb: &Ocb) -> Entry {
    match ocb {
        Ocb::MountDir { path, .. } => Entry {
            ino: pathmgr::node_ino(path) as u32,
            mode: imagefs::S_IFDIR | 0o555,
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        },
        Ocb::Text { .. } => Entry {
            ino: 1,
            mode: S_IFCHR | 0o600,
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        },
        Ocb::Sem(s) => Entry {
            ino: s.ino,
            mode: s.mode,
            uid: s.uid,
            gid: s.gid,
            mtime: 0,
            data: &[],
        },
        Ocb::SemDir { .. } => Entry {
            ino: 0x6000_0000 - 1,
            mode: imagefs::S_IFDIR | 0o555,
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        },
        Ocb::Tymem(_) => Entry {
            ino: 0x5000_0000,
            mode: 0x5000 | 0o666, // S_IFNAM
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        },
        Ocb::Shm(o) => {
            let s = super::shmem::stat(o);
            Entry {
                ino: s.st.st_ino as u32,
                mode: u32::from(s.st.st_mode),
                uid: 0,
                gid: 0,
                mtime: 0,
                data: &[],
            }
        }
        Ocb::Proc { node, .. } => super::procfs::entry(node).0,
        Ocb::Image { entry, .. } => *entry,
        Ocb::Dev { dev, .. } => Entry {
            ino: if *dev == Dev::Null {
                2
            } else {
                DEV_ZERO_INO as u32
            },
            mode: S_IFCHR | 0o666,
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        },
        Ocb::PathDir { path, .. } => {
            imagefs::get()
                .and_then(|fs| fs.entry(path))
                .unwrap_or(Entry {
                    ino: 1,
                    mode: imagefs::S_IFDIR | 0o555,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                    data: &[],
                })
        }
    }
}

/// `tymem`: for a `shm_ctl()` with `SHMCTL_TYMEM`, the typed memory object
/// open on the descriptor it names.
fn io_ocb(
    rcvid: u64,
    client: Client,
    ty: u16,
    msg: &[u8],
    total: usize,
    ocb: &mut Ocb,
    tymem: Option<super::tymem::Open>,
) -> Result<Answer, u32> {
    match (ty, ocb) {
        // pathconf(): none of procnto's files answers (EINVAL, as procnto).
        (IO_PATHCONF, _) => Err(EINVAL),
        (ty, Ocb::Shm(o))
            if !matches!(
                ty,
                IO_DEVCTL | IO_FDINFO | super::shmhandle::MEM_SHMEM_HANDLE | super::shmhandle::MEM_SHMEM_REVOKE
            ) =>
        {
            let (status, data) = super::shmem::io(rcvid, client.pid, ty, msg, total, o)?;
            Ok(Answer::Owned(status, data))
        }
        // `_IO_READ64`, `_IO_WRITE64`: only the devices and shared memory
        // take them (measured).
        (IO_READ64 | IO_WRITE64, Ocb::Image { .. } | Ocb::PathDir { .. } | Ocb::Proc { .. }) => Err(ENOSYS),
        (
            IO_READ,
            Ocb::Image {
                entry, path, pos, ..
            },
        ) if msg.len() >= 16 => {
            let nbytes = u32_at(msg, 4) as u64;
            let xtype = u32_at(msg, 8) & XTYPE_MASK;
            if entry.is_dir() {
                // readdir(): no extra per-entry data (XTYPE_READDIR flags);
                // a plain read() of a directory is EISDIR (procnto).
                if xtype == XTYPE_NONE {
                    return Err(EISDIR);
                }
                // pread(): EISDIR, EINVAL for an offset that doesn't fit
                // in 32 bits (measured).
                if xtype == XTYPE_OFFSET {
                    let at = u64::from_le_bytes(msg.get(16..24).ok_or(EINVAL)?.try_into().unwrap());
                    return Err(if at > u32::MAX as u64 { EINVAL } else { EISDIR });
                }
                if xtype != XTYPE_READDIR {
                    return Err(ENOSYS);
                }
                let (out, n) = dirents(client.pid, &image_names(path), *pos as usize, nbytes as usize);
                *pos += n as u64;
                return Ok(Answer::Owned(out.len() as u32, out));
            }
            // pread(): `struct _xtype_offset` after the message; the file
            // position stays.
            let at = match xtype {
                XTYPE_NONE => None,
                XTYPE_OFFSET if msg.len() >= 24 => {
                    Some(u64::from_le_bytes(msg[16..24].try_into().unwrap()))
                }
                _ => return Err(ENOSYS),
            };
            let data = entry.data;
            // A pread() past the end (or before the start): EINVAL.
            if at.is_some_and(|a| a > data.len() as u64) {
                return Err(EINVAL);
            }
            let start = (at.unwrap_or(*pos) as usize).min(data.len());
            let end = start.saturating_add(nbytes as usize).min(data.len());
            if at.is_none() {
                *pos = end as u64;
            }
            Ok(Answer::Static((end - start) as u32, &data[start..end]))
        }
        (IO_READ | IO_READ64, Ocb::Dev { dev, .. }) if msg.len() >= 16 => Ok(match dev {
            Dev::Null => Answer::Static(0, &[]),
            Dev::Klog => {
                let mut out = alloc::vec![0u8; (u32_at(msg, 4) as usize).min(crate::msgbuf::CAPACITY)];
                let n = crate::msgbuf::read(&mut out);
                if n == 0 {
                    return Err(crate::ker::errno::EAGAIN);
                }
                out.truncate(n);
                Answer::Owned(n as u32, out)
            }
            // The buffer starts zeroed and stays so.
            Dev::Zero => {
                let (n, data) = read_out(rcvid, u32_at(msg, 4) as usize, |_, piece| Ok(piece.len()))?;
                Answer::Owned(n as u32, data)
            }
        }),
        // Writes are discarded: all of it was written. Not open for
        // writing: EBADF.
        (IO_WRITE | IO_WRITE64, Ocb::Dev { ioflag, .. }) if *ioflag & IO_FLAG_WR == 0 => Err(EBADF),
        (IO_WRITE | IO_WRITE64, Ocb::Dev { .. }) if msg.len() >= 8 => Ok(Answer::Static(u32_at(msg, 4), &[])),
        // /dev/null stays at 0; /dev/zero has no position (ENOSYS).
        (IO_LSEEK, Ocb::Dev { dev: Dev::Null, .. }) => Ok(Answer::Owned(0, 0u64.to_le_bytes().to_vec())),
        (IO_LSEEK, Ocb::Dev { .. }) => Err(ENOSYS),
        (IO_READ | IO_READ64, Ocb::SemDir { pos, .. }) if msg.len() >= 16 => {
            let (out, n) = dirents(client.pid, &super::namedsem::names(), *pos as usize, u32_at(msg, 4) as usize);
            *pos += n as u64;
            Ok(Answer::Owned(out.len() as u32, out))
        }
        // The value as the size; `st_rdev` S_INSEM.
        (IO_STAT, Ocb::Sem(sem)) => {
            let mut s = stat(&ocb_entry(&Ocb::Sem(sem.clone())));
            s.st.st_size = sem.sem.value() as i64;
            s.st.st_rdev = u64::from(super::namedsem::S_INSEM);
            Ok(stat_answer(&s, client))
        }
        (IO_STAT, Ocb::Proc { node, .. }) => {
            Ok(stat_answer(&super::procfs::stat(node, client.pid), client))
        }
        (IO_STAT, Ocb::Image { entry, path, .. }) => Ok(stat_answer(&image_stat(entry, path), client)),
        (IO_STAT, Ocb::PathDir { path, .. }) if imagefs::get().and_then(|fs| fs.entry(path)).is_some() => {
            let e = imagefs::get().and_then(|fs| fs.entry(path)).unwrap();
            Ok(stat_answer(&image_stat(&e, path), client))
        }
        (IO_STAT, Ocb::Dev { dev, .. }) => {
            let (ino, rdev) = match dev {
                Dev::Null => (2, DEV_NULL_RDEV),
                Dev::Zero | Dev::Klog => (DEV_ZERO_INO as u32, DEV_ZERO_RDEV),
            };
            Ok(stat_answer(&dev_stat(ino, S_IFCHR | 0o666, 0, rdev), client))
        }
        // /proc/mount's directories: a size of 1, on device 1.
        (IO_STAT, ocb @ Ocb::MountDir { .. }) => {
            let mut s = stat(&ocb_entry(ocb));
            s.st.st_size = 1;
            s.st.st_dev = u64::from(DEVICES_DEV);
            s.st.st_nlink = 2;
            Ok(stat_answer(&s, client))
        }
        (IO_STAT, ocb) => Ok(stat_answer(&stat(&ocb_entry(ocb)), client)),
        // procnto's refusals on /proc (measured): `ctl` answers devctls
        // only (a pread() EINVAL); a write is EIO on /proc itself, EBADF
        // where the file isn't open for writing; chmod() is EIO on /proc,
        // the directories, `cmdline` and `exefile`, taken (and ignored) on
        // `as` and `ctl`, ENOSYS on `/proc/config`.
        (IO_READ | IO_WRITE | IO_LSEEK, Ocb::Proc { node, .. }) if super::procfs::is_ctl_file(node) => {
            Err(if ty == IO_READ && u32_at(msg, 8) & XTYPE_MASK != XTYPE_NONE { EINVAL } else { ENOSYS })
        }
        (IO_WRITE, Ocb::Proc { node: ProcNode::Root, .. }) => Err(EIO),
        (IO_WRITE, Ocb::Proc { node, ioflag, .. })
            if *ioflag & IO_FLAG_WR == 0 && !matches!(node, ProcNode::Config | ProcNode::Stats(_)) =>
        {
            Err(EBADF)
        }
        (IO_CHMOD, Ocb::Proc { node, .. }) => match node {
            ProcNode::Config | ProcNode::Stats(_) => Err(ENOSYS),
            ProcNode::File { .. } if super::procfs::is_ctl(node).is_some() => Ok(Answer::Static(0, &[])),
            _ => Err(EIO),
        },
        // `/proc/<pid>/as`: the process's memory, at the file position
        // (or at a pread()/pwrite() offset, the position unchanged).
        (IO_READ, Ocb::Proc { node, pos, .. })
            if msg.len() >= 16 && super::procfs::is_as(node).is_some() =>
        {
            let pid = super::procfs::is_as(node).unwrap();
            let at = xtype_offset(msg)?;
            let from = at.unwrap_or(*pos);
            // Up to the first unmapped byte (measured); from one, ESRVRFAULT.
            let (n, data) = read_out(rcvid, u32_at(msg, 4) as usize, |off, piece| {
                match super::procfs::as_read(pid, from.wrapping_add(off as u64), piece) {
                    Err(_) if off > 0 => Ok(0),
                    r => r,
                }
            })?;
            if at.is_none() {
                *pos += n as u64;
            }
            Ok(Answer::Owned(n as u32, data))
        }
        (IO_WRITE, Ocb::Proc { node, pos, .. })
            if msg.len() >= 16 && super::procfs::is_as(node).is_some() =>
        {
            let pid = super::procfs::is_as(node).unwrap();
            let at = xtype_offset(msg)?;
            let data_at = if at.is_some() { 24 } else { 16 };
            let to = at.unwrap_or(*pos);
            let n = write_in(rcvid, msg, total, data_at, u32_at(msg, 4) as usize, |off, piece| {
                super::procfs::as_write(pid, to.wrapping_add(off as u64), piece)
            })?;
            if at.is_none() {
                *pos += n as u64;
            }
            Ok(Answer::Static(n as u32, &[]))
        }
        (IO_READ, Ocb::Proc { node, pos, .. }) if msg.len() >= 16 => {
            let nbytes = u32_at(msg, 4) as usize;
            let xtype = u32_at(msg, 8) & XTYPE_MASK;
            if matches!(node, ProcNode::File { .. } | ProcNode::Config | ProcNode::Stats(_)) {
                // pread(): of `/proc/config` and the statistics only
                // (EINVAL, procnto).
                let at = match xtype_offset(msg)? {
                    Some(_) if !matches!(node, ProcNode::Config | ProcNode::Stats(_)) => return Err(EINVAL),
                    at => at,
                };
                let data = super::procfs::load(node);
                let start = (at.unwrap_or(*pos) as usize).min(data.len());
                let end = start
                    .saturating_add(nbytes)
                    .min(super::procfs::read_end(node, &data, start));
                if at.is_none() {
                    *pos = end as u64;
                }
                return Ok(Answer::Owned(
                    (end - start) as u32,
                    data[start..end].to_vec(),
                ));
            }
            // A directory: readdir() only (a plain read() or a pread() is
            // EISDIR).
            if xtype == XTYPE_NONE || xtype == XTYPE_OFFSET {
                return Err(EISDIR);
            }
            let (out, n) = dirents(client.pid, &super::procfs::names(node, client.pid), *pos as usize, nbytes);
            *pos += n as u64;
            Ok(Answer::Owned(out.len() as u32, out))
        }
        // The statistics have no position (ENOSYS, procnto's).
        (IO_LSEEK, Ocb::Proc { node: ProcNode::Stats(_), .. }) => Err(ENOSYS),
        (IO_LSEEK, Ocb::Proc { node, pos, .. }) if msg.len() >= 16 => {
            let whence = i16::from_le_bytes([msg[4], msg[5]]);
            let offset = i64::from_le_bytes(msg[8..16].try_into().unwrap());
            let base = match whence {
                SEEK_SET => 0,
                SEEK_CUR => *pos as i64,
                SEEK_END => super::procfs::seek_end(node) as i64,
                _ => return Err(EINVAL),
            };
            let new = base.checked_add(offset).filter(|&p| p >= 0).ok_or(EINVAL)?;
            // A process's directory, `cmdline` and `exefile` only rewind
            // (EINVAL otherwise, procnto's).
            let rewind_only = match node {
                ProcNode::Dir(_) => true,
                ProcNode::File { .. } => super::procfs::is_as(node).is_none(),
                _ => false,
            };
            if rewind_only && new != 0 {
                return Err(EINVAL);
            }
            *pos = new as u64;
            Ok(Answer::Owned(0, new.to_le_bytes().to_vec()))
        }
        (IO_LSEEK, Ocb::Image { entry, path, pos, .. }) if msg.len() >= 16 => {
            let whence = i16::from_le_bytes([msg[4], msg[5]]);
            let offset = i64::from_le_bytes(msg[8..16].try_into().unwrap());
            // A directory's position counts names (`dirents()`): FreeBSD's
            // `getdirentries()` asks for it after each read, and
            // `rewinddir()` sets it.
            let size = if entry.is_dir() { image_names(path).len() as i64 } else { entry.data.len() as i64 };
            let base = match whence {
                SEEK_SET => 0,
                SEEK_CUR => *pos as i64,
                SEEK_END => size,
                _ => return Err(EINVAL),
            };
            // Kept within the file (procnto's image filesystem).
            let new = base.saturating_add(offset).clamp(0, size);
            *pos = new as u64;
            Ok(Answer::Owned(0, new.to_le_bytes().to_vec()))
        }
        (IO_DEVCTL, ocb) if msg.len() >= 8 && u32_at(msg, 4) == DCMD_ALL_GETFLAGS => {
            // The open flags; procnto's directory handles add O_DIRECTORY.
            let flags = match ocb {
                Ocb::Image { ioflag, .. }
                | Ocb::Dev { ioflag, .. }
                | Ocb::Proc { ioflag, .. }
                | Ocb::SemDir { ioflag, .. }
                | Ocb::Text { ioflag }
                | Ocb::MountDir { ioflag, .. } => *ioflag,
                Ocb::Sem(_) => 0,
                Ocb::Shm(o) => super::shmem::ioflag(o),
                Ocb::Tymem(o) => o.ioflag,
                Ocb::PathDir { ioflag, .. } => *ioflag | O_DIRECTORY,
            };
            let flags = getfl_for(client.pid, flags);
            let mut r = alloc::vec![0u8; 20];
            r[8..12].copy_from_slice(&4u32.to_le_bytes());
            r[16..20].copy_from_slice(&flags.to_le_bytes());
            Ok(Answer::Owned(0, r))
        }
        // A devctl whose data is shorter than its `nbytes` says (procnto's
        // answers, measured, review PATHMEM-17): a shared memory file's
        // `F_SETFL` or `shm_ctl()` is EBADMSG; another file takes an
        // `F_SETFL` with no flags as done.
        (IO_DEVCTL, Ocb::Shm(_))
            if msg.len() >= 16
                && (u32_at(msg, 4) == DCMD_ALL_SETFLAGS || DCMD_MEMMGR_CTRL.contains(&u32_at(msg, 4)))
                && msg.len() - 16 < u32_at(msg, 8) as usize =>
        {
            Err(EBADMSG)
        }
        (IO_DEVCTL, _) if msg.len() >= 8 && msg.len() < 20 && u32_at(msg, 4) == DCMD_ALL_SETFLAGS => {
            Ok(Answer::Owned(0, alloc::vec![0u8; 16]))
        }
        // `fcntl(F_SETFL)`: O_APPEND, O_DSYNC, O_SYNC, O_RSYNC, O_NONBLOCK
        // and O_LARGEFILE as given (iofunc's rule); `/dev/null` and
        // `/dev/zero` answer it and keep their flags (measured, issue
        // #291), as `/dev/text` does (`devtext`).
        (IO_DEVCTL, ocb) if msg.len() >= 20 && u32_at(msg, 4) == DCMD_ALL_SETFLAGS => {
            // FreeBSD's: APPEND, DSYNC, SYNC, RSYNC, NONBLOCK, LARGEFILE.
            const SETTABLE: u32 = 0x8 | 0x100_0000 | 0x80 | 0x800_0000 | 0x4 | 0x1000_0000;
            let given = u32_at(msg, 16);
            let given = if native(client.pid) { given } else { qnx::ioflag_in(given & !7) };
            let new = given & SETTABLE;
            let set = |f: &mut u32| *f = (*f & !SETTABLE) | new;
            match ocb {
                Ocb::Image { ioflag, .. }
                | Ocb::Proc { ioflag, .. }
                | Ocb::SemDir { ioflag, .. }
                | Ocb::PathDir { ioflag, .. }
                | Ocb::Text { ioflag }
                | Ocb::MountDir { ioflag, .. } => set(ioflag),
                Ocb::Tymem(o) => set(&mut o.ioflag),
                Ocb::Shm(o) => super::shmem::with_ioflag(o, set),
                Ocb::Sem(_) | Ocb::Dev { .. } => {}
            }
            Ok(Answer::Owned(0, alloc::vec![0u8; 16]))
        }
        // `statvfs()`: procnto's image filesystem ("ifs", read-only and
        // trusted, a 4 KB block per 4 KB of image, its entries as files);
        // /dev/shmem all zeros but its name ("shmem"); ENOSYS for the
        // devices and /proc.
        (IO_DEVCTL, ocb) if msg.len() >= 8 && u32_at(msg, 4) == DCMD_FSYS_STATVFS => {
            let mut v = [0u8; MSG_STATVFS_SIZE];
            let basetype: &[u8] = match ocb {
                Ocb::Shm(_) => b"shmem",
                Ocb::Image { .. } | Ocb::PathDir { .. } => {
                    let fs = imagefs::get().ok_or(ENOTTY)?;
                    v[0..4].copy_from_slice(&4096u32.to_le_bytes()); // f_bsize
                    v[4..8].copy_from_slice(&4096u32.to_le_bytes()); // f_frsize
                    v[8..16].copy_from_slice(&(fs.size().div_ceil(4096) as u64).to_le_bytes());
                    v[32..40].copy_from_slice(&(fs.entries() as u64).to_le_bytes()); // f_files
                    v[56..60].copy_from_slice(&IFS_DEV.to_le_bytes()); // f_fsid
                    v[76..80].copy_from_slice(&(ST_RDONLY | ST_TRUSTED).to_le_bytes());
                    v[80..84].copy_from_slice(&256u32.to_le_bytes()); // f_namemax
                    b"ifs"
                }
                // procnto's devices and /proc: ENOSYS (measured).
                Ocb::Dev { .. } | Ocb::Proc { .. } => return Err(ENOSYS),
                _ => return Err(ENOTTY),
            };
            v[60..60 + basetype.len()].copy_from_slice(basetype);
            let mut r = alloc::vec![0u8; 16];
            r[8..12].copy_from_slice(&(MSG_STATVFS_SIZE as u32).to_le_bytes());
            r.extend_from_slice(&v);
            Ok(Answer::Owned(0, r))
        }
        // `struct _io_devctl` { type, combine_len, dcmd, nbytes, zero },
        // then the data; the reply is `struct _io_devctl_reply` { zero,
        // ret_val, nbytes, zero2 }, then the data.
        (IO_DEVCTL, Ocb::Proc { node, .. }) if msg.len() >= 16 => {
            let room = u32_at(msg, 8) as usize;
            let (ret, out) =
                super::procfs::devctl(node, client.pid, u32_at(msg, 4), &msg[16..], room)?;
            let mut r = alloc::vec![0u8; 16];
            r[4..8].copy_from_slice(&ret.to_le_bytes());
            r[8..12].copy_from_slice(&(out.len() as u32).to_le_bytes());
            r.extend_from_slice(&out);
            Ok(Answer::Owned(0, r))
        }
        // shm_ctl() (`DCMD_MEMMGR_CTRL`, 24 or 32 bytes of data).
        (IO_DEVCTL, Ocb::Shm(o)) if msg.len() >= 40 && DCMD_MEMMGR_CTRL.contains(&u32_at(msg, 4)) => {
            let ret = super::shmem::shm_ctl(o, &msg[16..], client.pid, tymem)?;
            let mut r = alloc::vec![0u8; 16];
            r[4..8].copy_from_slice(&ret.to_le_bytes());
            Ok(Answer::Owned(0, r))
        }
        // A devctl none of procnto's files knows: ENOSYS (libc's devctl()
        // and ioctl() make it ENOTTY; measured, review PATHMEM-17).
        (IO_DEVCTL, _) => Err(ENOSYS),
        (super::shmhandle::MEM_SHMEM_HANDLE, Ocb::Shm(o)) => {
            super::shmhandle::create(client.pid, o, msg).map(|r| Answer::Owned(0, r))
        }
        (super::shmhandle::MEM_SHMEM_REVOKE, Ocb::Shm(o)) => {
            super::shmhandle::revoke(client.pid, o, msg).map(|()| Answer::Static(0, &[]))
        }
        (super::shmhandle::MEM_SHMEM_HANDLE | super::shmhandle::MEM_SHMEM_REVOKE, _) => {
            Err(crate::ker::errno::EBADF)
        }
        // Not answered by /proc, /dev/sem and /proc/mount (measured).
        (IO_FDINFO, Ocb::Proc { .. } | Ocb::SemDir { .. } | Ocb::MountDir { .. }) => Err(ENOSYS),
        (IO_READ, Ocb::MountDir { path, pos, .. }) if msg.len() >= 16 => {
            if u32_at(msg, 8) & XTYPE_MASK != XTYPE_READDIR {
                return Err(EISDIR);
            }
            let names: Vec<(u64, String)> = pathmgr::mount_listing(path)
                .into_iter()
                .map(|n| {
                    let full = if path.is_empty() { n.clone() } else { alloc::format!("{path}/{n}") };
                    (pathmgr::node_ino(&full), n)
                })
                .collect();
            let (out, n) = dirents(client.pid, &names, *pos as usize, u32_at(msg, 4) as usize);
            *pos += n as u64;
            Ok(Answer::Owned(out.len() as u32, out))
        }
        (IO_FDINFO, ocb) => {
            let e = ocb_entry(ocb);
            let (path, ioflag, pos, size) = match ocb {
                Ocb::Image {
                    path,
                    ioflag,
                    pos,
                    entry,
                } => {
                    // A directory names the image's root (procnto's).
                    let path = if entry.is_dir() { "/".into() } else { format!("/{path}") };
                    (path, *ioflag, *pos, entry.data.len() as u64)
                }
                Ocb::PathDir { path, ioflag } if imagefs::get().is_some_and(|fs| fs.entry(path).is_some()) => {
                    ("/".into(), *ioflag, 0, 0)
                }
                Ocb::PathDir { path, ioflag } => (format!("/{path}"), *ioflag, 0, 0),
                Ocb::Dev { dev, ioflag } => {
                    let name = if *dev == Dev::Null {
                        "/dev/null"
                    } else {
                        "/dev/zero"
                    };
                    (name.into(), *ioflag, 0, 0)
                }
                Ocb::Tymem(o) => (alloc::format!("/{}", o.name), o.ioflag, 0, o.hi - o.lo + 1),
                Ocb::Sem(s) => ("/dev/sem".into(), 0, 0, s.sem.value() as u64),
                Ocb::SemDir { ioflag, .. } => ("/dev/sem".into(), *ioflag, 0, 0),
                Ocb::Text { ioflag } => ("/dev/text".into(), *ioflag, 0, 0),
                Ocb::MountDir { .. } => return Err(ENOSYS),
                Ocb::Shm(o) => {
                    let s = super::shmem::stat(o);
                    let pos = match o {
                        super::shmem::Open::File { pos, .. }
                        | super::shmem::Open::Dir { pos, .. } => *pos,
                    };
                    let (path, ioflag) = super::shmem::fdinfo_path(o);
                    (
                        path,
                        ioflag,
                        pos,
                        s.st.st_size as u64,
                    )
                }
                Ocb::Proc { node, ioflag, pos } => (
                    super::procfs::path(node),
                    *ioflag,
                    *pos,
                    super::procfs::entry(node).1,
                ),
            };
            let (status, r) = fdinfo(msg, e.mode, ioflag, pos, size, &path);
            Ok(Answer::Owned(status, r))
        }
        // Writes, chmod(), chown(), truncation and sync of these read-only
        // files and devices: ENOSYS, as procnto answers.
        (IO_WRITE | IO_WRITE64 | IO_CHMOD | IO_CHOWN | IO_SPACE | IO_SYNC, _) => Err(ENOSYS),
        // Types procnto's managers don't handle.
        (
            IO_RSVD_CLOSE_OCB | IO_NOTIFY | IO_RSVD_UNBLOCK | IO_UTIME | IO_SHUTDOWN | IO_MSG | IO_RSVD_CHECK_MSG
            | IO_RSVD_LOCK_OCB | IO_RSVD_UNLOCK_OCB | IO_POWER | IO_ACL | IO_RSVD_PAUSE | IO_RSVD_UNPAUSE
            | IO_NOTIFY64 | IO_UTIME64,
            _,
        ) => Err(ENOSYS),
        _ => {
            kprintln!(
                "[pid {}] resmgr: I/O message {ty:#06x} ({} bytes) not implemented",
                client.pid,
                msg.len()
            );
            Err(ENOSYS)
        }
    }
}

/// The least length of I/O message `ty`: `sizeof` its structure
/// (`<sys/iomsg.h>`), the 4-byte header for the reserved types; None for
/// types past `_IO_UTIME64`, which have none.
fn io_min_len(ty: u16) -> Option<usize> {
    Some(match ty {
        IO_READ | IO_WRITE | IO_DEVCTL | IO_LSEEK | IO_FDINFO | IO_READ64 | IO_WRITE64 => 16,
        IO_UTIME | IO_POWER | IO_ACL => 16,
        IO_STAT | IO_PATHCONF | IO_CHMOD | IO_SYNC | IO_MSG => 8,
        IO_CHOWN | IO_LOCK => 12,
        IO_SPACE => 24,
        IO_DUP | IO_UTIME64 => 40,
        IO_OPENFD => 48,
        IO_NOTIFY | IO_MMAP | IO_NOTIFY64 => 88,
        // _IO_CLOSE, _IO_SHUTDOWN, the reserved ones: the header alone.
        0x100..=IO_UTIME64 => 4,
        _ => return None,
    })
}

/// The check procnto's resource managers make before they look at an I/O
/// message: EBADMSG for a message shorter than its type's structure, or of
/// a type past `_IO_UTIME64` (measured, every manager alike).
pub fn io_check(ty: u16, len: usize) -> Result<(), u32> {
    match ty {
        0x101..=0x1ff if io_min_len(ty).is_none_or(|min| len < min) => Err(EBADMSG),
        _ => Ok(()),
    }
}

/// An I/O message (`_IO_*` other than `_IO_CONNECT`) on a connection bound
/// to one of procnto's resource managers.
pub fn io(rcvid: u64, client: Client, ty: u16, msg: &[u8]) {
    io_from(rcvid, client, ty, msg, msg.len())
}

/// `io()` of a message of which `msg` is the start, `total` bytes in all
/// in the client's buffers: a write's data past `msg` is read from there
/// as it is written (`write_in`).
pub fn io_from(rcvid: u64, client: Client, ty: u16, msg: &[u8], total: usize) {
    if let Err(e) = io_check(ty, msg.len()) {
        return error(rcvid, e);
    }
    // A context closed here goes after the table's lock (`release`).
    let closed: Option<Ocb>;
    let mut ocbs = OCBS.lock();
    let r = match ty {
        IO_CLOSE => match {
            closed = ocbs.remove(&client).map(|o| o.lock().clone());
            &closed
        } {
            // The debugger's last close of a process's `as`/`ctl` lets it
            // go (it opens and closes others' to list them).
            Some(Ocb::Proc { node, .. }) => {
                let still_open = |pid| {
                    ocbs.iter().any(|(c, o)| {
                        c.pid == client.pid
                            && matches!(&*o.lock(), Ocb::Proc { node, .. } if super::procfs::is_ctl(node) == Some(pid))
                    })
                };
                if let Some(pid) = super::procfs::is_ctl(node)
                    && !still_open(pid)
                    && let Some(p) = crate::ker::process::lookup(pid)
                    && p.debug
                        .lock()
                        .debugger
                        .is_some_and(|(d, _)| d == client.pid)
                {
                    crate::ker::debug::detach(&p);
                }
                Ok(Answer::Static(0, &[]))
            }
            // Closing a file lets go of the process's locks on it.
            Some(Ocb::Shm(o)) => {
                if let Some((key, ..)) = super::shmem::lock_target(o) {
                    super::locks::release(key, client.pid);
                }
                Ok(Answer::Static(0, &[]))
            }
            Some(_) => Ok(Answer::Static(0, &[])),
            // The connection of a resmgr_attach(): detach the name, with
            // the table free: the PATHSPACE event that raises may be a
            // SIGEV_SEM, found through the table (review PATHMEM-2).
            None => {
                drop(ocbs);
                let r = if pathmgr::detach(client) { Ok(Answer::Static(0, &[])) } else { Err(EBADF) };
                return answer(rcvid, r);
            }
        },
        // openfd() (on the new connection): ENOSYS; EBADF for no access
        // mode (measured).
        IO_OPENFD if u32_at(msg, 4) & O_ACCMODE == 0 => Err(EBADF),
        IO_OPENFD => Err(ENOSYS),
        // Record locks (fcntl()): on /dev/shmem files.
        IO_LOCK => match ocbs.get(&client).map(|o| o.lock().clone()) {
            Some(Ocb::Shm(o)) => match super::shmem::lock_target(&o) {
                Some((key, pos, size)) => super::locks::lock(key, client.pid, msg, pos, size)
                    .map(|data| Answer::Owned(0, data)),
                None => Err(EINVAL),
            },
            Some(_) => Err(crate::ker::errno::ENOSYS),
            None => Err(EBADF),
        },
        // dup(): the new connection gets (a copy of) the context of the
        // one in the message's `struct _msg_info_hdr`: the client's own,
        // or another process's that it passed on (`ConnectDupFd()`).
        IO_DUP if msg.len() >= DUP_COID + 4 => {
            let coid = u32_at(msg, DUP_COID) as i32;
            let pid = match u32_at(msg, DUP_PID) as i32 {
                0 => client.pid,
                pid => pid,
            };
            if pid != client.pid
                && !crate::ker::dupfd::authorized(
                    pid,
                    coid,
                    client.pid,
                    crate::ker::process::KERNEL_PID,
                )
            {
                return error(rcvid, crate::ker::errno::EPERM);
            }
            let from = Client { pid, coid };
            // `.cloned()` clones the `Arc`: the new connection shares the
            // context (its position, its flags), it doesn't get a copy.
            match ocbs.get(&from).cloned() {
                // Not over a context the new connection still has (`unbound`).
                Some(_) if ocbs.contains_key(&client) => Err(EBUSY),
                Some(ocb) => {
                    ocbs.insert(client, ocb);
                    Ok(Answer::Static(0, &[]))
                }
                None => Err(EBADF),
            }
        }
        _ => {
            // shm_ctl()'s typed memory descriptor (`paddr`), looked up here,
            // where all the connections are at hand.
            let tymem = (ty == IO_DEVCTL && msg.len() >= 40 && u32_at(msg, 32) & 0x2000 != 0)
                .then(|| Client { pid: client.pid, coid: u32_at(msg, 16) as i32 })
                .and_then(|c| match ocbs.get(&c).map(|o| o.lock().clone()) {
                    Some(Ocb::Tymem(t)) => Some(t),
                    _ => None,
                });
            match ocbs.get(&client).cloned() {
                // The operation runs under the context's own lock, with
                // the table free (review PATHMEM-2): what it does may
                // deliver an event, and a SIGEV_SEM one is found through
                // the table. An event it raises under the context's lock
                // (`shm_ctl`'s CONTIG_ALLOC_FAIL) goes out after. A long
                // one runs on a copy of the context with no lock held, as
                // the lock masks interrupts (`long_io`).
                Some(ocb) => {
                    drop(ocbs);
                    let copy = {
                        let o = ocb.lock();
                        long_io(ty, msg, &o).then(|| o.clone())
                    };
                    let r = match copy {
                        // Nothing else uses the context meanwhile: process
                        // 1 has one thread for messages (`sysmgr::server`),
                        // so the channel's queue serializes a context's I/O
                        // as procnto's per-OCB mutex does. With a pool of
                        // threads, this would need a gate that blocks.
                        Some(mut copy) => {
                            let r = io_ocb(rcvid, client, ty, msg, total, &mut copy, tymem);
                            put_pos(&mut ocb.lock(), &copy);
                            r
                        }
                        None => io_ocb(rcvid, client, ty, msg, total, &mut ocb.lock(), tymem),
                    };
                    super::procevent::deliver_later();
                    return answer(rcvid, r);
                }
                // The connection of a resmgr_attach(): resmgr_pathname()
                // asks it for the attached name.
                // resmgr_devino() fstat()s it: st_dev 1 and an inode of the
                // name's own, which drivers give their devices (procnto,
                // measured; it was EBADF and their inodes 0, review
                // PATHMEM-25).
                None if ty == IO_STAT => match pathmgr::attached_ino(client) {
                    Some(ino) => {
                        let mut s = dev_stat(0, 0, 0, 0);
                        s.st.st_ino = ino as u64;
                        Ok(stat_answer(&s, client))
                    }
                    None => Err(EBADF),
                },
                None if ty == IO_FDINFO => match pathmgr::attached_name(client) {
                    Some(name) => {
                        let (status, r) = fdinfo(msg, 0, 0, 0, 0, &name);
                        Ok(Answer::Owned(status, r))
                    }
                    None => Err(EBADF),
                },
                None => Err(EBADF),
            }
        }
    };
    drop(ocbs);
    answer(rcvid, r)
}

/// `shm_ctl()`'s devctl: `__DIOT(_DCMD_PROC, __PROC_SUBCMD_MEMMGR + 0,
/// ...)` (`<sys/dcmd_proc.h>`), 24 bytes of data (`DCMD_MEMMGR_MEMOBJ_OLD`)
/// or 32 (`DCMD_MEMMGR_MEMOBJ`, with the special flags).
const DCMD_MEMMGR_CTRL: [u32; 2] = [0x8018_0864, 0x8020_0864];

/// Whether an operation on `ocb` may take long: a shared memory file's
/// read, write or `shm_ctl()` (which zero-fills all it allocates), a read
/// or write of `/proc/<pid>/as`. Those run on a copy of the context, not
/// under its lock: the lock masks interrupts, while procnto's thread stays
/// preemptible throughout (measured, review PATHMEM-4: a 512 MiB
/// `shm_ctl()` kept a 1 ms timer 1.9 s late on its CPU; procnto's, 0.3
/// ms). Only the position comes back (`put_pos`).
fn long_io(ty: u16, msg: &[u8], ocb: &Ocb) -> bool {
    match ocb {
        Ocb::Shm(super::shmem::Open::File { .. }) => {
            matches!(ty, IO_READ | IO_READ64 | IO_WRITE | IO_WRITE64)
                || ty == IO_DEVCTL && msg.len() >= 8 && DCMD_MEMMGR_CTRL.contains(&u32_at(msg, 4))
        }
        Ocb::Proc { node, .. } => matches!(ty, IO_READ | IO_WRITE) && super::procfs::is_as(node).is_some(),
        _ => false,
    }
}

/// Bring the position a long operation moved (`long_io`) back into the
/// context it ran a copy of.
fn put_pos(ocb: &mut Ocb, copy: &Ocb) {
    match (ocb, copy) {
        (Ocb::Shm(super::shmem::Open::File { pos, .. }), Ocb::Shm(super::shmem::Open::File { pos: moved, .. }))
        | (Ocb::Proc { pos, .. }, Ocb::Proc { pos: moved, .. }) => *pos = *moved,
        _ => {}
    }
}

/// Reply to an I/O message with its answer.
fn answer(rcvid: u64, r: Result<Answer, u32>) {
    match r {
        Ok(Answer::Static(status, data)) => reply(rcvid, status, data),
        Ok(Answer::Owned(status, data)) => reply(rcvid, status, &data),
        Err(e) => error(rcvid, e),
    }
}

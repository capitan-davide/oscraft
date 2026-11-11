//! procnto's path manager (QNX `pathmgr/`): resolves pathnames to the
//! servers that have attached a prefix of them.
//!
//! `open()` and friends start by sending an `_IO_CONNECT` message
//! (`<sys/iomsg.h>`) to `PATHMGR_PID`/`PATHMGR_CHID` (procnto, chid 1) with
//! handle `PATHMGR_HANDLE` (0). The path manager answers with a link reply,
//! status `_IO_CONNECT_RET_FLAG | _IO_CONNECT_RET_LINK`: a
//! `struct _io_connect_link_reply`, one `struct _io_connect_entry` per
//! server whose prefix matches (deepest first) and the path made absolute.
//! libc then connects to each server in turn and sends the `_IO_CONNECT`
//! again, with the entry's handle and the path less the server's prefix
//! (`prefix_len` bytes), until one of them accepts. A server can also
//! answer with a link reply without entries: a symbolic link, whose new
//! path libc resolves from the start.
//!
//! Relative paths are resolved against the process's working directory,
//! which procnto keeps (`_PATH_CHDIR`, sent by `chdir()`), absolute ones
//! against its root directory (`_PATH_CHROOT`, `chroot()`), above which
//! ".." doesn't lead. Both are kept as real paths.
//!
//! procnto attaches its own names (the image filesystem at `/`, see
//! `resmgr`, and the text console, see `devtext`); other servers attach
//! theirs with `resmgr_attach()` (see `attach`).

use alloc::string::String;
use alloc::vec::Vec;

use super::resmgr::{self, Client};
use crate::ker::errno::{EEXIST, EINVAL, EISDIR, ENAMETOOLONG, ENOENT};
use crate::ker::process::{self, DEVTEXT_CHID, KERNEL_PID, SYSMGR_CHID};
use crate::sync::IrqSpinLock;

pub const IO_CONNECT: u16 = 0x0100;
/// `struct _io_connect` up to `path`.
pub const CONNECT_HDR: usize = 40;
const PATHMGR_HANDLE: u32 = 0;
/// `_IO_FLAG_RD | _IO_FLAG_WR | _IO_FLAG_EX`
const IO_FLAG_MASK: u32 = 0x7;
// connect reply status bits
pub const CONNECT_RET_FLAG: u32 = 0x8000_0000;
pub const CONNECT_RET_LINK: u32 = 0x0001_0000;
/// `_IO_CONNECT_RET_MSG`: attach to the server in the reply's
/// `struct _server_info` (and send it the message that follows, if any).
const CONNECT_RET_MSG: u32 = 0x0010_0000;
/// `_IO_CONNECT_RET_UMASK`: the client takes the reply's umask out of the
/// mode it sends on (libc's `_connect_ctrl`, read with REA), so a server
/// creating a file needn't know it.
const CONNECT_RET_UMASK: u32 = 0x0002_0000;
/// `sizeof(struct _server_info32)`.
const SERVER_INFO32_SIZE: usize = 48;
const LINK_REPLY_SIZE: usize = 20;
const ENTRY_SIZE: usize = 32;
/// `_FTYPE_ANY`, `_FTYPE_SHMEM`, `_FTYPE_TYMEM`
const FTYPE_ANY: u32 = 0;
const FTYPE_SHMEM: u32 = 5;
const FTYPE_TYMEM: u32 = 13;
/// `_FTYPE_NAME`: `name_attach()`'s names (`/dev/name/local/...`).
const FTYPE_NAME: u32 = 12;
/// OsCraft's: the QNX personality's service (`qnxd`) attaches itself with
/// it, as QNX's dumper with `_FTYPE_DUMPER`.
const FTYPE_QNX: u32 = 0x4f53;

// PATHMGR_FLAG_* (<sys/pathmgr.h>)
const FLAG_BEFORE: u32 = 0x1;
const FLAG_AFTER: u32 = 0x2;
/// Paths below the prefix don't go on to the servers of shorter ones.
const FLAG_OPAQUE: u32 = 0x4;
const FLAG_FTYPEONLY: u32 = 0x8;
const FLAG_STICKY: u32 = 0x20;
const FLAG_DIR: u32 = 0x100;
/// A process's own names resolve for it only with this.
const FLAG_SELF: u32 = 0x200;
/// `_IO_CONNECT_MKNOD`: mkdir(), mkfifo(), mknod().
const CONNECT_MKNOD: u16 = 5;
/// `_IO_CONNECT_LINK` and `_IO_CONNECT_EXTRA_RESMGR_LINK`.
const CONNECT_LINK: u16 = 7;
const EXTRA_RESMGR_LINK: u8 = 7;
/// `struct _io_resmgr_link_extra`: nd, pid, chid, handle, flags,
/// file_type, reserved[2].
const RESMGR_LINK_SIZE: usize = 32;

// ANCHOR: mount
/// A server attached at a path prefix (`resmgr_attach()`).
#[derive(Clone)]
struct Mount {
    /// Without leading or trailing '/'; "" is the root.
    prefix: String,
    pid: i32,
    chid: i32,
    handle: u32,
    file_type: u32,
    /// `PATHMGR_FLAG_*`: `DIR` also resolves the paths below the prefix,
    /// `FTYPEONLY` only connects of its file type.
    flags: u32,
    /// The connection that attached it: closing it (or the process
    /// ending) detaches the name. None for procnto's own names.
    owner: Option<Client>,
    /// When it was attached (directories list the newest names first).
    seq: u64,
}
// ANCHOR_END: mount

/// The next attachment's `seq`.
static SEQ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// The path space's nodes (the directories implied by the attached
/// prefixes, the attached names themselves, the directories of
/// `/proc/mount`), each with an ID for life: its index here, given when
/// the path is first seen and never reused, so that a handle in a link
/// reply and the inode numbers `stat()` and `readdir()` report stay
/// valid across attaches and detaches (review PATHMEM-7; procnto's are
/// its node structures' addresses, as stable).
static NODES: IrqSpinLock<Vec<String>> = IrqSpinLock::new(Vec::new());

/// The ID of the node at `path` (without leading '/'), given now if new.
pub fn node_id(path: &str) -> u32 {
    let mut nodes = NODES.lock();
    match nodes.iter().position(|n| n == path) {
        Some(i) => i as u32,
        None => {
            nodes.push(path.into());
            (nodes.len() - 1) as u32
        }
    }
}

/// The path of node `id`, if there is one.
pub fn node_path(id: u32) -> Option<String> {
    NODES.lock().get(id as usize).cloned()
}

/// The inode number of path-space node `path`: `stat()` and `readdir()`
/// agree on it.
pub fn node_ino(path: &str) -> u64 {
    resmgr::MOUNT_NODE_HANDLE as u64 | node_id(path) as u64
}

/// The attached names: at the same prefix, the earlier entries are tried
/// first.
static MOUNTS: IrqSpinLock<Vec<Mount>> = IrqSpinLock::new(Vec::new());

fn builtin(prefix: &str, chid: i32, handle: u32, flags: u32) -> Mount {
    Mount {
        prefix: prefix.into(),
        pid: KERNEL_PID,
        chid,
        handle,
        file_type: FTYPE_ANY,
        flags,
        owner: None,
        seq: 0,
    }
}

/// Attach procnto's own names: the image filesystem at `/`, the text
/// console (`devtext`) under each of its names, `/dev/tty` (the session's
/// controlling terminal, see `session`), `/dev/null`, `/dev/zero` and the
/// process filesystem at `/proc`; shared memory objects at `/dev/shmem`,
/// and at `/` for connects of `_FTYPE_SHMEM` (`shm_open()` connects the
/// object's name as it is); named semaphores likewise, at `/dev/sem` and
/// for `_FTYPE_SEM`.
pub fn init() {
    let mut m = MOUNTS.lock();
    m.push(Mount {
        file_type: FTYPE_SHMEM,
        ..builtin(
            "",
            SYSMGR_CHID,
            resmgr::SHMEM_HANDLE,
            FLAG_DIR | FLAG_FTYPEONLY,
        )
    });
    m.push(Mount {
        file_type: FTYPE_TYMEM,
        ..builtin(
            "",
            SYSMGR_CHID,
            resmgr::TYMEM_HANDLE,
            FLAG_DIR | FLAG_FTYPEONLY,
        )
    });
    m.push(Mount {
        file_type: super::namedsem::FTYPE_SEM,
        ..builtin(
            "",
            SYSMGR_CHID,
            resmgr::SEM_HANDLE,
            FLAG_DIR | FLAG_FTYPEONLY,
        )
    });
    m.push(builtin("", SYSMGR_CHID, resmgr::IMAGEFS_HANDLE, FLAG_DIR));
    for name in ["dev/text", "dev/console"] {
        m.push(builtin(name, DEVTEXT_CHID, 0, 0));
    }
    m.push(builtin("dev/tty", SYSMGR_CHID, resmgr::DEV_TTY_HANDLE, 0));
    m.push(builtin("dev/null", SYSMGR_CHID, resmgr::DEV_NULL_HANDLE, 0));
    m.push(builtin("dev/zero", SYSMGR_CHID, resmgr::DEV_ZERO_HANDLE, 0));
    m.push(builtin("dev/klog", SYSMGR_CHID, resmgr::DEV_KLOG_HANDLE, 0));
    m.push(builtin(
        "dev/shmem",
        SYSMGR_CHID,
        resmgr::SHMEM_HANDLE,
        FLAG_DIR,
    ));
    m.push(builtin("dev/sem", SYSMGR_CHID, resmgr::SEM_HANDLE, FLAG_DIR));
    m.push(builtin(
        "proc",
        SYSMGR_CHID,
        resmgr::PROCFS_HANDLE,
        FLAG_DIR,
    ));
}

fn mounts() -> Vec<Mount> {
    MOUNTS.lock().clone()
}

/// `_IO_CONNECT_LINK` with a `struct _io_resmgr_link_extra`
/// (`resmgr_attach()`, libc's `pathmgr_link()`), first step: libc sends it
/// on the connection all processes share with procnto, and keeps the
/// connection the final answer comes from as the attachment's ID (closed
/// by `resmgr_detach()`). So send libc back to procnto on a connection of
/// its own, see `attach`.
fn link_redirect(client: Client, rcvid: u64, c: &Connect) -> Result<(), u32> {
    // Attaching a name takes PROCMGR_AID_PATHSPACE (as procnto).
    if !process::lookup(client.pid).is_some_and(|p| crate::ker::ability::allowed(&p, crate::ker::ability::PATHSPACE, None)) {
        return Err(crate::ker::errno::EPERM);
    }
    let full = alloc::format!("/{}", absolute(client.pid, c.path));
    // libc follows entries of the connect's own file type (or _FTYPE_ALL).
    let entry = (
        KERNEL_PID,
        SYSMGR_CHID,
        resmgr::LINK_HANDLE,
        c.file_type,
        0,
    );
    link_reply(rcvid, c, &[entry], &full)
}

/// `_IO_CONNECT_LINK` second step, on the link's own connection (see
/// `link_redirect`): attach a server's name, bound to that connection
/// (unless `PATHMGR_FLAG_STICKY`).
fn attach(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    let x = c.combined;
    if x.len() < RESMGR_LINK_SIZE {
        return Err(EINVAL);
    }
    let int = |o: usize| u32_at(x, o);
    let flags = int(16);
    let m = Mount {
        prefix: absolute(client.pid, c.path),
        pid: match int(4) as i32 {
            0 => client.pid,
            pid => pid,
        },
        chid: int(8) as i32,
        handle: int(12),
        flags,
        file_type: int(20),
        owner: (flags & FLAG_STICKY == 0).then_some(client),
        seq: SEQ.fetch_add(1, core::sync::atomic::Ordering::Relaxed),
    };
    if m.file_type == FTYPE_QNX {
        crate::ker::msg::set_qnxd(m.pid, m.chid)?;
    }
    let hash = path_hash(&m.prefix);
    {
        let mut mounts = MOUNTS.lock();
        // A name_attach() name exists once (procnto: EEXIST).
        if m.file_type == FTYPE_NAME && mounts.iter().any(|o| o.file_type == FTYPE_NAME && o.prefix == m.prefix) {
            return Err(EEXIST);
        }
        // The newest attachment at a prefix is tried first, unless it asks
        // to come after the others.
        if flags & FLAG_BEFORE != 0 || flags & FLAG_AFTER == 0 {
            mounts.insert(0, m);
        } else {
            mounts.push(m);
        }
    }
    super::procevent::trigger(super::procevent::PATHSPACE, hash);
    resmgr::reply(rcvid, 0, &[]);
    Ok(())
}

/// Detach the names attached through a connection; returns whether there
/// were any (its `_IO_CLOSE`).
pub fn detach(client: Client) -> bool {
    detach_where(|m| m.owner == Some(client))
}

/// Detach the names `which` selects, with a `PATHSPACE` event for each;
/// returns whether there were any.
fn detach_where(which: impl Fn(&Mount) -> bool) -> bool {
    let mut gone = Vec::new();
    MOUNTS.lock().retain(|m| {
        let detach = which(m);
        if detach {
            gone.push(path_hash(&m.prefix));
        }
        !detach
    });
    for &hash in &gone {
        super::procevent::trigger(super::procevent::PATHSPACE, hash);
    }
    !gone.is_empty()
}

/// The `PATHSPACE` event's hash of an attached prefix (its absolute path).
fn path_hash(prefix: &str) -> u32 {
    super::procevent::pathspace_hash(&alloc::format!("/{prefix}"))
}

/// The name attached with connection `client` (absolute), if any.
pub fn attached_name(client: Client) -> Option<String> {
    MOUNTS
        .lock()
        .iter()
        .find(|m| m.owner == Some(client))
        .map(|m| alloc::format!("/{}", m.prefix))
}

/// The inode number of the name attached through `client`, for an
/// `fstat()` of that connection: what `resmgr_devino()` asks, and what
/// drivers (io-char's) then give their devices as `st_ino`. Unique to the
/// attachment and never reused (its `seq`); procnto's is its link
/// structure's address, as stable (measured, review PATHMEM-25).
pub fn attached_ino(client: Client) -> Option<u64> {
    MOUNTS
        .lock()
        .iter()
        .find(|m| m.owner == Some(client))
        .map(|m| LINK_INO | m.seq)
}

/// `attached_ino`'s numbers, apart from the path-space nodes' and the
/// files'.
const LINK_INO: u64 = 0x4000_0000_0000_0000;

/// Detach the names attached by a process that has ended.
pub fn process_exited(pid: i32) {
    detach_where(|m| m.owner.is_some_and(|o| o.pid == pid));
}

/// The fields of `struct _io_connect` that procnto's servers look at.
pub struct Connect<'a> {
    pub subtype: u16,
    pub file_type: u32,
    pub reply_max: u16,
    pub handle: u32,
    pub ioflag: u32,
    pub mode: u32,
    pub eflag: u8,
    pub extra_type: u8,
    /// Directory a relative path starts from (the `*at()` calls); 0: the
    /// working directory.
    pub dirfd: u16,
    pub path: &'a str,
    /// The I/O message combined with the connect (`_IO_CONNECT_COMBINE`,
    /// `_IO_CONNECT_COMBINE_CLOSE`): the extra data.
    pub combined: &'a [u8],
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

pub fn parse(msg: &[u8], client: i32) -> Option<Connect<'_>> {
    if msg.len() < CONNECT_HDR {
        return None;
    }
    let path_len = u16_at(msg, 34) as usize;
    let path = msg.get(CONNECT_HDR..CONNECT_HDR + path_len)?;
    let path = path.split(|&c| c == 0).next()?;
    let extra_len = u16_at(msg, 38) as usize;
    // The extra data (for a combine, the I/O message) follows the path,
    // aligned to 8 bytes (`_QNX_MSG_ALIGN`).
    let extra = (CONNECT_HDR + path_len).next_multiple_of(8);
    Some(Connect {
        subtype: u16_at(msg, 2),
        file_type: u32_at(msg, 4),
        reply_max: u16_at(msg, 8),
        handle: u32_at(msg, 16),
        ioflag: if client == crate::ker::process::KERNEL_PID || resmgr::native(client) { u32_at(msg, 20) } else { crate::personality::qnx::ioflag_in(u32_at(msg, 20)) },
        mode: u32_at(msg, 24),
        eflag: msg[36],
        extra_type: msg[37],
        dirfd: u16_at(msg, 32),
        path: core::str::from_utf8(path).ok()?,
        combined: msg.get(extra..extra + extra_len).unwrap_or(&[]),
    })
}

/// The names in directory `dir` (absolute, without leading '/') under
/// which servers are attached (e.g. "dev" in "", "tty" in "dev").
pub fn mount_names(dir: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // The newest first, as procnto lists them.
    let mut ms = mounts();
    ms.sort_by_key(|m| core::cmp::Reverse(m.seq));
    for m in ms {
        let rest = if dir.is_empty() {
            Some(m.prefix.as_str())
        } else {
            m.prefix.strip_prefix(dir).and_then(|r| r.strip_prefix('/'))
        };
        if let Some(name) = rest.and_then(|r| r.split('/').next())
            && !name.is_empty()
            && !out.iter().any(|n| n == name)
        {
            out.push(name.into());
        }
    }
    out
}

/// The directories that exist because servers are attached below them
/// (the proper prefixes of the mount points, e.g. "dev"). The path
/// manager's handle for one is `DIR_NODE_HANDLE + node_id`.
pub fn dir_nodes() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in mounts() {
        for (i, _) in m.prefix.match_indices('/') {
            let dir = &m.prefix[..i];
            if !out.iter().any(|d| d == dir) {
                out.push(dir.into());
            }
        }
    }
    out
}

// ANCHOR: covers
/// Does mount `m` cover `path` (without leading '/') for a connect of
/// file type `file_type`?
fn covers(m: &Mount, path: &str, file_type: u32) -> bool {
    // A name_attach() name takes only `_FTYPE_NAME` connects
    // (name_open()); procnto answers the others (`pathmgr/pathmgr_node.c`).
    if (m.flags & FLAG_FTYPEONLY != 0 || m.file_type == FTYPE_NAME) && m.file_type != file_type {
        return false;
    }
    let dir = m.flags & FLAG_DIR != 0;
    if m.prefix.is_empty() {
        return dir || path.is_empty();
    }
    match path.strip_prefix(m.prefix.as_str()) {
        Some("") => true,
        Some(rest) => dir && rest.starts_with('/'),
        None => false,
    }
}
// ANCHOR_END: covers

/// The names attached with `name_attach()` (absolute, without leading
/// '/'). The path manager's handle for one is `NAME_NODE_HANDLE +
/// node_id`.
pub fn name_nodes() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in mounts().into_iter().filter(|m| m.file_type == FTYPE_NAME) {
        if !out.contains(&m.prefix) {
            out.push(m.prefix);
        }
    }
    out
}

/// The name (without the leading '/') a server's channel is attached
/// under.
pub fn name_of(pid: i32, chid: i32) -> Option<String> {
    mounts()
        .into_iter()
        .find(|m| m.pid == pid && m.chid == chid && m.flags & FLAG_FTYPEONLY == 0)
        .map(|m| m.prefix)
}

/// The server another server's name resolves to, deepest first: (pid,
/// chid, handle, the path relative to its prefix). For procnto's own
/// clients (the boot script); the image filesystem doesn't count.
pub fn lookup(path: &str) -> Option<(i32, i32, u32, String)> {
    let path = path.trim_matches('/');
    let mut found: Vec<Mount> = mounts()
        .into_iter()
        .filter(|m| !m.prefix.is_empty() && covers(m, path, FTYPE_ANY))
        .collect();
    found.sort_by_key(|m| core::cmp::Reverse(m.prefix.len()));
    let m = found.into_iter().next()?;
    let rest = path[m.prefix.len()..].trim_start_matches('/');
    Some((m.pid, m.chid, m.handle, rest.into()))
}

/// A link reply: `entries` (nd 0) and `path`, status
/// `_IO_CONNECT_RET_FLAG | _IO_CONNECT_RET_LINK`. Without entries it
/// describes a symbolic link.
pub fn link_reply(
    rcvid: u64,
    c: &Connect,
    entries: &[(i32, i32, u32, u32, u16)],
    path: &str,
) -> Result<(), u32> {
    link_reply_status(rcvid, c, entries, path, CONNECT_RET_FLAG | CONNECT_RET_LINK | CONNECT_RET_UMASK)
}

/// The contents of a symbolic link for `readlink()`: the same reply, but
/// status 0, so the client reads the path instead of following it
/// (procnto's `image_link_redirect` without redirection).
pub fn readlink_reply(rcvid: u64, c: &Connect, target: &str) -> Result<(), u32> {
    link_reply_status(rcvid, c, &[], target, 0)
}

/// A connect answered with a server to attach to
/// (`_IO_CONNECT_RET_MSG`): the link reply's header (no entries, no
/// message), then the server's `struct _server_info32` (nd, srcnd, pid,
/// tid, chid, ...).
fn server_reply(rcvid: u64, c: &Connect, pid: i32, chid: i32) -> Result<(), u32> {
    let mut r = alloc::vec![0u8; LINK_REPLY_SIZE + SERVER_INFO32_SIZE];
    r[4..8].copy_from_slice(&c.file_type.to_le_bytes());
    r[8] = c.eflag;
    let info = LINK_REPLY_SIZE;
    r[info + 8..info + 12].copy_from_slice(&pid.to_le_bytes());
    r[info + 16..info + 20].copy_from_slice(&chid.to_le_bytes());
    resmgr::reply(rcvid, CONNECT_RET_FLAG | CONNECT_RET_MSG, &r);
    Ok(())
}

fn link_reply_status(
    rcvid: u64,
    c: &Connect,
    entries: &[(i32, i32, u32, u32, u16)],
    path: &str,
    status: u32,
) -> Result<(), u32> {
    let mut r = Vec::with_capacity(LINK_REPLY_SIZE + entries.len() * ENTRY_SIZE + path.len() + 1);
    r.extend_from_slice(&0u32.to_le_bytes()); // reserved1
    r.extend_from_slice(&c.file_type.to_le_bytes());
    r.extend_from_slice(&[c.eflag, 0]);
    r.extend_from_slice(&0u16.to_le_bytes()); // chroot_len
    // The client's umask (`_IO_CONNECT_RET_UMASK`).
    let umask = u32::try_from(rcvid)
        .ok()
        .and_then(crate::sched::thread)
        .map_or(0, |t| t.process.umask.load(core::sync::atomic::Ordering::Relaxed));
    r.extend_from_slice(&umask.to_le_bytes());
    r.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    r.extend_from_slice(&(path.len() as u16 + 1).to_le_bytes());
    for &(pid, chid, handle, file_type, prefix_len) in entries {
        r.extend_from_slice(&0u32.to_le_bytes()); // nd
        r.extend_from_slice(&pid.to_le_bytes());
        r.extend_from_slice(&chid.to_le_bytes());
        r.extend_from_slice(&handle.to_le_bytes());
        r.extend_from_slice(&0u32.to_le_bytes());
        r.extend_from_slice(&file_type.to_le_bytes());
        r.extend_from_slice(&prefix_len.to_le_bytes());
        r.extend_from_slice(&[0; 6]);
    }
    r.extend_from_slice(path.as_bytes());
    r.push(0);
    if r.len() > c.reply_max as usize {
        return Err(ENAMETOOLONG);
    }
    resmgr::reply(rcvid, status, &r);
    Ok(())
}

/// `path` made absolute against process `pid`'s working directory (or,
/// for an absolute path, its root directory), with "." and ".." resolved
/// (lexically) and without the leading '/'.
pub fn absolute(pid: i32, path: &str) -> String {
    absolute_at(pid, 0, path)
}

// ANCHOR: overlay
/// Where QNX's programs and libraries are, as FreeBSD keeps Linux's in
/// `/compat/linux`.
const COMPAT: &str = "compat/qnx";

/// The path a QNX program means: its own `/bin/sh`, in `/compat/qnx`, if
/// there is one, else the system's (`/dev`, `/tmp`, ...), as FreeBSD's
/// Linux emulation looks paths up.
fn overlay(pid: i32, path: String) -> String {
    let qnx = process::lookup(pid).is_some_and(|p| p.personality() == process::Personality::Qnx);
    if !qnx || path.is_empty() || path == COMPAT || path.starts_with("compat/qnx/") {
        return path;
    }
    let inside = alloc::format!("{COMPAT}/{path}");
    if super::imagefs::get().is_some_and(|fs| fs.entry(&inside).is_some()) { inside } else { path }
}
// ANCHOR_END: overlay

/// `absolute`, with a relative path starting from directory descriptor
/// `dirfd` of process `pid` if it is one of procnto's directories.
pub fn absolute_at(pid: i32, dirfd: u16, path: &str) -> String {
    let lexical = lexical_at(pid, dirfd, path);
    overlay(pid, lexical)
}

fn lexical_at(pid: i32, dirfd: u16, path: &str) -> String {
    let dir = (dirfd != 0)
        .then(|| resmgr::path_dir(pid, dirfd as i32))
        .flatten();
    let p = process::lookup(pid);
    let root = p.as_ref().map_or(String::new(), |p| p.root.lock().clone());
    let cwd = match (path.starts_with('/'), dir) {
        (true, _) => root.clone(),
        (false, Some(dir)) => dir,
        (false, None) => p.map_or(String::new(), |p| p.cwd.lock().clone()),
    };
    let root: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    let mut parts: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            // Not above the root.
            ".." if parts == root => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// `_PATH_CHDIR`: the header of `struct _io_connect`, then the new working
/// directory (libc's `chdir()` has checked that it is one). libc sends the
/// path its check resolved to: the real path (see `resolve`), so the root
/// directory doesn't apply.
pub const PATH_CHDIR: u16 = 0x0071;

pub fn chdir(rcvid: u64, client: Client, msg: &[u8]) {
    match parse(msg, client.pid).and_then(|c| Some((process::lookup(client.pid)?, c))) {
        Some((p, c)) => {
            *p.cwd.lock() = absolute(0, c.path);
            resmgr::reply(rcvid, 0, &[]);
        }
        None => resmgr::error(rcvid, EINVAL),
    }
}

/// `_PATH_CHROOT`: the header of `struct _io_connect`, then the new root
/// directory (libc's `chroot()` has checked that it is one), its real path
/// as for `_PATH_CHDIR`.
pub const PATH_CHROOT: u16 = 0x0072;

pub fn chroot(rcvid: u64, client: Client, msg: &[u8]) {
    match parse(msg, client.pid).and_then(|c| Some((process::lookup(client.pid)?, c))) {
        Some((p, _)) if !crate::ker::ability::allowed(&p, crate::ker::ability::CHROOT, None) => {
            resmgr::error(rcvid, crate::ker::errno::EPERM)
        }
        Some((p, c)) => {
            *p.root.lock() = absolute(0, c.path);
            resmgr::reply(rcvid, 0, &[]);
        }
        None => resmgr::error(rcvid, EINVAL),
    }
}

/// `_PATH_OPENDIR`: the header of `struct _io_connect` and a path. Opens a
/// directory handle on the connection the message came on, for the
/// `*at()` calls (procnto's `pathmgr_dir_open`: no access mode allowed).
pub const PATH_OPENDIR: u16 = 0x0074;

pub fn opendir(rcvid: u64, client: Client, msg: &[u8]) {
    let Some(c) = parse(msg, client.pid) else {
        return resmgr::error(rcvid, EINVAL);
    };
    if c.ioflag & IO_FLAG_MASK != 0 {
        return resmgr::error(rcvid, EISDIR);
    }
    if let Err(e) = resmgr::unbound(client) {
        return resmgr::error(rcvid, e);
    }
    let path = absolute(client.pid, c.path);
    resmgr::bind_path_dir(client, path, c.ioflag);
    resmgr::reply(rcvid, 0, &[]);
}

/// `_IO_CONNECT_EFLAG_*`: what the path's end said, which resolving it
/// (`absolute`) loses. The client passes them on to the server.
pub const EFLAG_DIR: u8 = 0x01;
const EFLAG_DOT: u8 = 0x02;
const EFLAG_DOTDOT: u8 = 0x04;

/// The `_IO_CONNECT_EFLAG_*` of `path`: it names a directory if it ends
/// with '/', "." or "..".
fn path_eflag(path: &str) -> u8 {
    match path.rsplit('/').next() {
        Some("") if !path.is_empty() => EFLAG_DIR,
        Some(".") => EFLAG_DIR | EFLAG_DOT,
        Some("..") => EFLAG_DIR | EFLAG_DOT | EFLAG_DOTDOT,
        _ => 0,
    }
}

/// A server's name in `/proc/mount`: `nd,pid,chid,handle,file_type`.
fn leaf_name(m: &Mount) -> String {
    alloc::format!("0,{},{},{},{}", m.pid, m.chid, m.handle, m.file_type)
}

/// The directories of `/proc/mount` (procnto's node for one has handle
/// `MOUNT_NODE_HANDLE + node_id`): the root, then every path a server is
/// attached at or below.
pub fn mount_dirs() -> Vec<String> {
    let mut out: Vec<String> = alloc::vec![String::new()];
    for m in mounts() {
        let mut at = 0;
        while at <= m.prefix.len() && !m.prefix.is_empty() {
            let end = m.prefix[at..].find('/').map_or(m.prefix.len(), |i| at + i);
            let dir = &m.prefix[..end];
            if !out.iter().any(|d| d == dir) {
                out.push(dir.into());
            }
            at = end + 1;
        }
    }
    out
}

/// The names in `/proc/mount/<dir>`, as procnto lists them: the servers
/// attached at `dir`, newest first, then the names below it.
pub fn mount_listing(dir: &str) -> Vec<String> {
    let mut ms = mounts();
    ms.sort_by_key(|m| core::cmp::Reverse(m.seq));
    let mut out: Vec<String> = ms.iter().filter(|m| m.prefix == dir).map(leaf_name).collect();
    out.extend(mount_names(dir));
    out
}

/// `/proc/mount/<rest>`, procnto's view of the path space by server: a
/// path the servers are attached at or below is a directory procnto
/// serves; in it, a server attached there by its `leaf_name`, the rest of
/// the path going to that server alone (measured).
fn resolve_mount(rcvid: u64, c: &Connect, rest: &str) -> Result<(), u32> {
    let comps: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    let full = if comps.is_empty() {
        String::from("/proc/mount")
    } else {
        alloc::format!("/proc/mount/{}", comps.join("/"))
    };
    let dirs = mount_dirs();
    let mut dir = String::new();
    let mut consumed = "/proc/mount".len();
    for (i, comp) in comps.iter().enumerate() {
        consumed += 1 + comp.len();
        if let Some(m) = mounts().into_iter().find(|m| m.prefix == dir && leaf_name(m) == *comp) {
            let prefix_len = consumed + usize::from(i + 1 < comps.len());
            return link_reply(rcvid, c, &[(m.pid, m.chid, m.handle, m.file_type, prefix_len as u16)], &full);
        }
        dir = if dir.is_empty() { (*comp).into() } else { alloc::format!("{dir}/{comp}") };
        if !dirs.contains(&dir) {
            return Err(ENOENT);
        }
    }
    if !dirs.contains(&dir) {
        return Err(ENOENT);
    }
    let node = (KERNEL_PID, SYSMGR_CHID, resmgr::MOUNT_NODE_HANDLE + node_id(&dir), FTYPE_ANY, full.len() as u16);
    link_reply(rcvid, c, &[node], &full)
}

/// Resolve `c.path` to the servers attached to its prefixes.
fn resolve(rcvid: u64, client: Client, c: &Connect) -> Result<(), u32> {
    let c = &Connect {
        eflag: c.eflag | path_eflag(c.path),
        ..*c
    };
    let path = &absolute_at(client.pid, c.dirfd, c.path);
    if let Some(rest) = path.strip_prefix("proc/mount")
        && (rest.is_empty() || rest.starts_with('/'))
    {
        return resolve_mount(rcvid, c, rest);
    }
    // name_open(): procnto answers with the name's server, which libc
    // attaches to without a message (as procnto: the server doesn't see
    // the connect).
    if c.file_type == FTYPE_NAME {
        let server = MOUNTS
            .lock()
            .iter()
            .find(|m| m.file_type == FTYPE_NAME && m.prefix == *path)
            .map(|m| (m.pid, m.chid));
        let (pid, chid) = server.ok_or(ENOENT)?;
        return server_reply(rcvid, c, pid, chid);
    }
    // The servers whose prefix covers the path (no copy of the table: a
    // lookup per open()), as (pid, chid, handle, file type, prefix length).
    let (mut found, is_node): (Vec<(i32, i32, u32, u32, usize)>, bool) = {
        let mounts = MOUNTS.lock();
        // As procnto: a process's own names don't resolve for it (unless
        // attached with PATHMGR_FLAG_SELF).
        let covering = || {
            mounts
                .iter()
                .filter(|m| covers(m, path, c.file_type))
                .filter(|m| m.pid != client.pid || m.flags & FLAG_SELF != 0)
        };
        // An opaque server hides the servers of shorter prefixes, not
        // the others at its own (procnto, measured).
        let opaque = covering()
            .filter(|m| m.flags & FLAG_OPAQUE != 0)
            .map(|m| m.prefix.len())
            .max()
            .unwrap_or(0);
        let found = covering()
            .filter(|m| m.prefix.len() >= opaque)
            .map(|m| (m.pid, m.chid, m.handle, m.file_type, m.prefix.len()))
            .collect();
        // A directory that only exists in the path space (e.g. /dev): some
        // prefix goes on below it.
        let is_node = !path.is_empty()
            && mounts.iter().any(|m| {
                m.prefix
                    .strip_prefix(path.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            });
        (found, is_node)
    };
    // Deepest first; the sort is stable, so the order at a prefix stays.
    found.sort_by_key(|m| core::cmp::Reverse(m.4));
    // Such a directory is served by the path manager itself, before the
    // servers of its parents.
    let node = (is_node && dir_nodes().iter().any(|n| n == path)).then(|| node_id(path));
    // A name_attach() name, for a connect of another type: procnto's own
    // node (an S_IFNAM entry), before the servers of its parents.
    let name_node =
        (c.file_type != FTYPE_NAME && name_nodes().iter().any(|n| n == path)).then(|| node_id(path));
    if found.is_empty() && node.is_none() && name_node.is_none() {
        return Err(ENOENT);
    }
    // mkdir(), mkfifo(): procnto's pathmgr_domknod() answers for a name a
    // server is attached at (ENOENT, or EEXIST with names below it) and for
    // a path below a name_attach() name (ENOENT); the server never sees it.
    // procfs answers for its own names.
    if c.subtype == CONNECT_MKNOD && !path.is_empty() {
        if name_node.is_none() && name_nodes().iter().any(|n| path.strip_prefix(n.as_str()).is_some_and(|r| r.starts_with('/'))) {
            return Err(ENOENT);
        }
        let at = |m: &&(i32, i32, u32, u32, usize)| m.4 == path.len() && !(m.0 == KERNEL_PID && m.2 == resmgr::PROCFS_HANDLE);
        // The image's directories are the path manager's nodes too.
        let image_dir = super::imagefs::get().and_then(|fs| fs.entry(path)).is_some_and(|e| e.is_dir());
        if found.iter().any(|m| at(&m)) {
            return Err(if is_node || image_dir { EEXIST } else { ENOENT });
        }
    }
    // The root itself is "" (and its server's prefix empty).
    let full = if path.is_empty() {
        String::new()
    } else {
        alloc::format!("/{path}")
    };
    let node_entry = node.map(|id| {
        let handle = resmgr::DIR_NODE_HANDLE + id;
        (
            KERNEL_PID,
            SYSMGR_CHID,
            handle,
            FTYPE_ANY,
            full.len() as u16,
        )
    });
    let name_entry = name_node.map(|id| {
        (
            KERNEL_PID,
            SYSMGR_CHID,
            resmgr::NAME_NODE_HANDLE + id,
            FTYPE_ANY,
            full.len() as u16,
        )
    });
    // A name is a leaf that only procnto serves: nothing after it.
    if let Some(e) = name_entry {
        return link_reply(rcvid, c, &[e], &full);
    }
    let entry = |&(pid, chid, handle, file_type, len): &(i32, i32, u32, u32, usize)| {
        // The prefix with its leading '/', and the '/' after it if the
        // path goes on.
        let prefix_len = match len {
            0 if path.is_empty() => 0,
            0 => 1,
            n if path.len() > n => n + 2,
            n => n + 1,
        };
        (pid, chid, handle, file_type, prefix_len as u16)
    };
    // Servers attached at the path itself, then the path-space directory,
    // then the servers of its parents (as procnto).
    let (at, parents): (Vec<_>, Vec<_>) = found.iter().partition(|m| m.4 == path.len() && !path.is_empty());
    let entries: Vec<_> = at
        .iter()
        .map(|m| entry(m))
        .chain(node_entry)
        .chain(parents.iter().map(|m| entry(m)))
        .collect();
    link_reply(rcvid, c, &entries, &full)
}

/// Handle an `_IO_CONNECT` received on process 1's channel.
pub fn connect(rcvid: u64, client: Client, msg: &[u8]) {
    let r = match parse(msg, client.pid) {
        None => Err(EINVAL),
        Some(c)
            if c.handle == PATHMGR_HANDLE
                && c.subtype == CONNECT_LINK
                && c.extra_type == EXTRA_RESMGR_LINK =>
        {
            link_redirect(client, rcvid, &c)
        }
        Some(c)
            if c.handle == resmgr::LINK_HANDLE
                && c.subtype == CONNECT_LINK
                && c.extra_type == EXTRA_RESMGR_LINK =>
        {
            attach(rcvid, client, &c)
        }
        Some(c) if c.handle == PATHMGR_HANDLE => resolve(rcvid, client, &c),
        Some(c) => resmgr::connect(rcvid, client, &c),
    };
    if let Err(e) = r {
        resmgr::error(rcvid, e);
    }
}

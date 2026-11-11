//! The POSIX layer of a server, `<sys/iofunc.h>`: what a file is
//! (`iofunc_attr_t`), what an open file is (`iofunc_ocb_t`), and the
//! default functions that give a server POSIX semantics: permissions,
//! sharing, offsets, times, `stat()`, `lseek()`, `chmod()` and the rest.
//! A server fills its function tables with these defaults
//! (`iofunc_func_init()`) and replaces the ones it handles itself.

use core::ffi::{c_int, c_long, c_uint, c_void};
use core::ptr::null_mut;

use kercall::neutrino::connect::{ClientInfoExtFree, ConnectClientInfo_r};
use kercall::neutrino::msg::MsgDeliverEvent_r;
use kercall::tls;

use crate::abi::*;
use crate::acl::iofunc_acl_default;
use crate::resmgr::{_resmgr_iofuncs, resmgr_devino, resmgr_open_bind, resmgr_pathname};
use core::mem::offset_of;
use crate::compat::{ConnectClientInfoAble, ConnectClientInfoExt};
use proto::connect::Connect;
use proto::errno::{EINTEGRITY, EMSGSIZE, ENOTDIR, ENOTEMPTY, ESPIPE, EXDEV};
use proto::fcntl::{
    O_EXEC, STATUS,
    O_APPEND, O_CREAT, O_DSYNC, O_EXCL, O_LARGEFILE, O_NOFOLLOW, O_NONBLOCK, O_RSYNC, O_SYNC,
    O_TRUNC,
};
use proto::io::{self, Header, ReadWrite};
use proto::pod::Pod;
use proto::stat::{S_IFBLK, S_IFDIR, S_IFIFO, S_IFLNK, S_IFMT, S_IFNAM, S_IFREG, QnxStat};

/// Reads a received message's header, a `T` of `proto`'s, at `msg`.
pub(crate) fn hdr<T: Pod>(msg: *const u8) -> T {
    at(msg, 0)
}

/// Reads a `T` at byte `off` of a received message: data after a header.
pub(crate) fn at<T: Pod>(msg: *const u8, off: usize) -> T {
    // SAFETY: the framework reads only what is within the message it has
    // checked, or within its buffer; any bytes are a T.
    unsafe { msg.wrapping_add(off).cast::<T>().read_unaligned() }
}

/// Writes a header back into the message buffer, for the reply.
fn put<T: Pod>(msg: *mut u8, v: &T) {
    // SAFETY: as at(): the buffer holds the header.
    unsafe { msg.cast::<T>().write_unaligned(*v) }
}

/// Writes a `T` at byte `off` of the message buffer.
pub(crate) fn put_at<T: Pod>(msg: *mut u8, off: usize, v: T) {
    // SAFETY: as at().
    unsafe { msg.wrapping_add(off).cast::<T>().write_unaligned(v) }
}

/// The permission bits `iofunc_check_access()` checks, and its two
/// checks of ownership: the owner (`S_ISUID`) and the group (`S_ISGID`).
const S_ISUID: u32 = 0x800;
const S_ISGID: u32 = 0x400;
const S_IREAD: u32 = 0x100;
const S_IWRITE: u32 = 0x80;
const S_IEXEC: u32 = 0x40;
const S_IXALL: u32 = 0o111;

/// `_IO_FLAG_RD`, `_IO_FLAG_WR`: an OCB's access, in `ioflag`'s low bits.
const IO_FLAG_RD: i32 = 1;
const IO_FLAG_WR: i32 = 2;
/// `IOFUNC_ATTR_*`: what changed, and must be written or stamped.
const ATTR_MTIME: u32 = 0x1;
const ATTR_ATIME: u32 = 0x2;
const ATTR_CTIME: u32 = 0x4;
const ATTR_DIRTY_OWNER: u32 = 0x20;
const ATTR_DIRTY_TIME: u32 = 0x40;
const ATTR_DIRTY_MODE: u32 = 0x80;
const ATTR_DIRTY_NLINK: u32 = 0x100;
const ATTR_DIRTY_RDEV: u32 = 0x200;
const ATTR_NS_TIMESTAMPS: u32 = 0x1000;
const ATTR_SYNTHETIC: u32 = 0x8000;
/// `IOFUNC_PC_*`, a mount's configuration.
const PC_CHOWN_RESTRICTED: u32 = 0x1;
const PC_SYNC_IO: u32 = 0x4;
const PC_EXT: u32 = 0x20;
/// `IOFUNC_MOUNT_32BIT`, and `ST_RDONLY` among a mount's flags.
const MOUNT_32BIT: u32 = 0x100;
const ST_RDONLY: u32 = 0x1;
/// `IOFUNC_OCB_MMAP`.
const OCB_MMAP: u16 = 0x2;
/// The `IOFUNC_OCB_LAST_*` results of `iofunc_ocb_detach()`.
const LAST_INUSE: c_int = 0x1;
const LAST_READER: c_int = 0x2;
const LAST_WRITER: c_int = 0x4;
const LAST_RDLOCK: c_int = 0x8;
const LAST_WRLOCK: c_int = 0x10;
/// `_IO_XTYPE_*`: how a read or write is qualified.
const XTYPE_NONE: u32 = 0;
const XTYPE_READCOND: u32 = 1;
const XTYPE_OFFSET: u32 = 5;
const XTYPE_READDIR: u32 = 9;
const XFLAG_NONBLOCK: u32 = 0x4000;
const XFLAG_BLOCK: u32 = 0x8000;
/// The largest offset without `O_LARGEFILE`.
const OFF32_MAX: i64 = 0x7fff_ffff;

/// An I/O default as an entry of a table: the same function, with the
/// table's untyped signature.
macro_rules! entry {
    ($f:ident) => {{
        unsafe extern "C" fn e(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
            // SAFETY: called by the framework with a message and the
            // OCB the binding holds, which the defaults take as
            // iofunc_ocb_t.
            unsafe { $f(ctp, msg, ocb.cast()) }
        }
        e
    }};
}

/// The default connect functions: only open.
static DEFAULT_CONNECT: [ConnectFn; CONNECT_NFUNCS] = {
    let mut t: [ConnectFn; CONNECT_NFUNCS] = [None; CONNECT_NFUNCS];
    t[0] = Some(open_default_entry);
    t
};

/// The default I/O functions, by message type from `_IO_READ`.
static DEFAULT_IO: [IoFn; IO_NFUNCS] = {
    let mut t: [IoFn; IO_NFUNCS] = [None; IO_NFUNCS];
    t[0x00] = Some(entry!(iofunc_read_default));
    t[0x01] = Some(entry!(iofunc_write_default));
    t[0x02] = Some(entry!(iofunc_close_ocb_default));
    t[0x03] = Some(entry!(iofunc_stat_default));
    t[0x05] = Some(entry!(iofunc_devctl_default));
    t[0x06] = Some(entry!(iofunc_unblock_default));
    t[0x07] = Some(entry!(iofunc_pathconf_default));
    t[0x08] = Some(entry!(iofunc_lseek_default));
    t[0x09] = Some(entry!(iofunc_chmod_default));
    t[0x0a] = Some(entry!(iofunc_chown_default));
    t[0x0b] = Some(entry!(iofunc_utime_default));
    t[0x0c] = Some(entry!(iofunc_openfd_default));
    t[0x0d] = Some(entry!(iofunc_fdinfo_default));
    t[0x0e] = Some(entry!(iofunc_lock_default));
    t[0x11] = Some(entry!(iofunc_mmap_default));
    t[0x15] = Some(entry!(iofunc_close_dup_default));
    t[0x16] = Some(entry!(iofunc_lock_ocb_default));
    t[0x17] = Some(entry!(iofunc_unlock_ocb_default));
    t[0x18] = Some(entry!(iofunc_sync_default));
    t[0x19] = Some(entry!(iofunc_power_default));
    t[0x1a] = Some(entry!(iofunc_acl_default));
    t[0x20] = Some(entry!(iofunc_utime_default));
    t
};

/// Fills the tables with the defaults, and sets their sizes: up to the
/// sizes given, which a server compiled with older headers has smaller.
///
/// # Safety
///
/// `connect` and `io` are null or tables of at least the sizes given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_func_init(
    nconnect: c_uint,
    connect: *mut ConnectFuncs,
    nio: c_uint,
    io: *mut IoFuncs,
) {
    if nconnect != 0 && !connect.is_null() {
        let n = (nconnect as usize).min(CONNECT_NFUNCS);
        // SAFETY: the caller's guarantee.
        let t = unsafe { &mut *connect };
        t.nfuncs = n as c_uint;
        t.funcs[..n].copy_from_slice(&DEFAULT_CONNECT[..n]);
    }
    if nio != 0 && !io.is_null() {
        let n = (nio as usize).min(IO_NFUNCS);
        // SAFETY: the caller's guarantee.
        let t = unsafe { &mut *io };
        t.nfuncs = n as c_uint;
        t.funcs[..n].copy_from_slice(&DEFAULT_IO[..n]);
    }
}

/// `iofunc_open_default()` as a connect function.
unsafe extern "C" fn open_default_entry(
    ctp: *mut Context,
    msg: *mut u8,
    handle: *mut c_void,
    extra: *mut c_void,
) -> c_int {
    // SAFETY: the framework's call, with the link's handle as the
    // attribute.
    unsafe { iofunc_open_default(ctp, msg, handle.cast(), extra) }
}

/// The default `check_msg`: a device command or private message may not
/// be combined.
pub fn check_msg(t: u16, combined: bool) -> c_int {
    if (t == IO_DEVCTL || t == IO_MSG) && combined {
        EBADMSG
    } else {
        0
    }
}

/// `iofunc_check_msg_default()`.
///
/// # Safety
///
/// `msg` is an I/O message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_check_msg_default(
    _ctp: *const Context,
    msg: *const u8,
    combine: bool,
    _ocb: *const c_void,
) -> c_int {
    // SAFETY: the caller's guarantee.
    check_msg(hdr::<Header>(msg).r#type, combine)
}

/// `PTHREAD_RMUTEX_INITIALIZER`'s words with `_NTO_SYNC_WAKEUP`: the
/// attribute's lock, recursive, initialised by the kernel on first use.
// FreeBSD's PTHREAD_MUTEX_INITIALIZER: a null pointer, which libthr fills on first use.
const ATTR_LOCK_INIT: [u32; 2] = [0, 0];

/// Initialises `attr` as a file of `mode`, owned by the client of `info`
/// (or the server), in the directory `dattr` (if not null), whose mount,
/// and with the set-group-ID bit, group, it shares.
///
/// # Safety
///
/// `attr` is writable for `size` bytes, at least an `iofunc_attr_t`;
/// `dattr` and `info` are null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_attr_init_sized(
    attr: *mut IofuncAttr,
    mode: u32,
    dattr: *const IofuncAttr,
    info: *const ClientInfo,
    size: usize,
) {
    // SAFETY: the caller's guarantee.
    unsafe {
        attr.cast::<u8>()
            .write_bytes(0, size.max(size_of::<IofuncAttr>()))
    };
    // SAFETY: as above.
    let a = unsafe { &mut *attr };
    // SAFETY: as above.
    let (uid, gid) = match unsafe { info.as_ref() } {
        Some(i) => (i.euid, i.egid),
        None => {
            let me = kercall::process::credentials();
            (me.ruid, me.rgid)
        }
    };
    a.flags = ATTR_NS_TIMESTAMPS | ATTR_MTIME | ATTR_ATIME | ATTR_CTIME | ATTR_DIRTY_TIME;
    a.uid = uid;
    a.gid = gid;
    a.mode = mode;
    // SAFETY: as above.
    if let Some(d) = unsafe { dattr.as_ref() } {
        a.mount = d.mount;
        if d.mode & S_ISGID != 0 {
            a.gid = d.gid;
            if mode & S_IFMT == S_IFDIR {
                a.mode |= S_ISGID;
            }
        }
    }
    a.nlink = 1;
    a.lock = ATTR_LOCK_INIT;
}

/// `iofunc_attr_init_sized()` for an `iofunc_attr_t`.
///
/// # Safety
///
/// As `iofunc_attr_init_sized()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_attr_init(
    attr: *mut IofuncAttr,
    mode: u32,
    dattr: *const IofuncAttr,
    info: *const ClientInfo,
) {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_attr_init_sized(attr, mode, dattr, info, size_of::<IofuncAttr>()) }
}

unsafe extern "C" {
    fn pthread_mutex_lock(m: *mut c_void) -> c_int;
    fn pthread_mutex_unlock(m: *mut c_void) -> c_int;
    fn pthread_mutex_trylock(m: *mut c_void) -> c_int;
}

/// Takes the attribute's lock.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_attr_lock(attr: *mut IofuncAttr) -> c_int {
    // SAFETY: the caller's guarantee: the lock is a pthread_mutex_t.
    let lock = unsafe { &raw mut (*attr).lock };
    // SAFETY: as above: the lock is a pthread_mutex_t.
    unsafe { pthread_mutex_lock(lock.cast()) }
}

/// Releases the attribute's lock.
///
/// # Safety
///
/// `attr` is locked by the caller.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_attr_unlock(attr: *mut IofuncAttr) -> c_int {
    // SAFETY: the caller's guarantee.
    let lock = unsafe { &raw mut (*attr).lock };
    // SAFETY: as above: the lock is a pthread_mutex_t.
    unsafe { pthread_mutex_unlock(lock.cast()) }
}

/// Takes the attribute's lock if it is free.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_attr_trylock(attr: *mut IofuncAttr) -> c_int {
    // SAFETY: the caller's guarantee.
    let lock = unsafe { &raw mut (*attr).lock };
    // SAFETY: as above: the lock is a pthread_mutex_t.
    unsafe { pthread_mutex_trylock(lock.cast()) }
}

/// The mount's functions, if it has a table.
fn mount_funcs(attr: &IofuncAttr) -> Option<&IofuncFuncs> {
    // SAFETY: the mount, if any, outlives the attribute.
    let mount = unsafe { attr.mount.as_ref() }?;
    // SAFETY: as above: its table.
    unsafe { mount.funcs.as_ref() }
}

/// The attribute's lock and unlock functions: the mount's, or the
/// defaults.
fn lockers(
    attr: &IofuncAttr,
) -> (
    unsafe extern "C" fn(*mut IofuncAttr) -> c_int,
    unsafe extern "C" fn(*mut IofuncAttr) -> c_int,
) {
    let f = mount_funcs(attr);
    let lock = f
        .filter(|f| f.nfuncs >= 3)
        .and_then(|f| f.attr_lock)
        .unwrap_or(iofunc_attr_lock);
    let unlock = f
        .filter(|f| f.nfuncs >= 4)
        .and_then(|f| f.attr_unlock)
        .unwrap_or(iofunc_attr_unlock);
    (lock, unlock)
}

/// The mount's OCB allocator and freer, or the defaults.
fn ocb_funcs(
    attr: &IofuncAttr,
) -> (
    unsafe extern "C" fn(*mut Context, *mut IofuncAttr) -> *mut IofuncOcb,
    unsafe extern "C" fn(*mut IofuncOcb),
) {
    let f = mount_funcs(attr);
    let calloc = f
        .filter(|f| f.nfuncs >= 1)
        .and_then(|f| f.ocb_calloc)
        .unwrap_or(iofunc_ocb_calloc);
    let free = f
        .filter(|f| f.nfuncs >= 2)
        .and_then(|f| f.ocb_free)
        .unwrap_or(iofunc_ocb_free);
    (calloc, free)
}

/// The mount's flags, 0 without a mount.
fn mount_flags(attr: &IofuncAttr) -> u32 {
    // SAFETY: the mount outlives the attribute.
    unsafe { attr.mount.as_ref() }.map_or(0, |m| m.flags)
}

/// The mount's configuration, `IOFUNC_PC_*`.
fn mount_conf(attr: &IofuncAttr) -> Option<u32> {
    // SAFETY: as above.
    unsafe { attr.mount.as_ref() }.map(|m| m.conf)
}

/// Whether the attribute may not change: a read-only mount.
fn read_only(attr: &IofuncAttr) -> bool {
    mount_flags(attr) & ST_RDONLY != 0 && attr.flags & ATTR_SYNTHETIC == 0
}

/// The largest offset an OCB may reach.
fn max_offset(ocb: &IofuncOcb, attr: &IofuncAttr) -> i64 {
    if ocb.ioflag as u32 & O_LARGEFILE == 0 || mount_flags(attr) & MOUNT_32BIT != 0 {
        OFF32_MAX
    } else {
        i64::MAX
    }
}

/// Initialises a mount of `size` bytes: extended (`IOFUNC_PC_EXT`).
///
/// # Safety
///
/// `mount` is writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mount_init(mount: *mut IofuncMount, size: usize) -> c_int {
    if size < 0x30 {
        return crate::dispatch::fail(EINVAL);
    }
    // SAFETY: the caller's guarantee.
    unsafe { mount.cast::<u8>().write_bytes(0, size) };
    // SAFETY: as above.
    let m = unsafe { &mut *mount };
    m.size = size;
    m.conf = PC_EXT;
    0
}

/// Clears a mount `iofunc_mount_init()` initialised.
///
/// # Safety
///
/// `mount` is such a mount.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mount_fini(mount: *mut IofuncMount) -> c_int {
    // SAFETY: the caller's guarantee.
    let m = unsafe { &mut *mount };
    if m.conf & PC_EXT == 0 {
        return crate::dispatch::fail(EINVAL);
    }
    let size = m.size;
    // SAFETY: as above.
    unsafe { mount.cast::<u8>().write_bytes(0, size) };
    0
}

/// Sets the mount's time resolution, in nanoseconds.
///
/// # Safety
///
/// `mount` is an extended mount.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mount_set_time(mount: *mut IofuncMount, timeres: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    let m = unsafe { &mut *mount };
    if m.conf & PC_EXT == 0 || timeres == 0 {
        return crate::dispatch::fail(EINVAL);
    }
    m.timeres = timeres;
    m.ext_flags |= 1;
    0
}

/// `O_REALIDS`: the client's real IDs, not its effective ones, count.
const O_REALIDS: c_int = proto::fcntl::O_REALIDS as c_int;

/// Swaps the real and effective IDs, for `O_REALIDS`.
fn real_ids(info: &mut ClientInfo, ioflag: c_int) {
    if ioflag & O_REALIDS != 0 {
        core::mem::swap(&mut info.ruid, &mut info.euid);
        core::mem::swap(&mut info.rgid, &mut info.egid);
    }
}

/// The client's credentials, into `info`, with room for 8 groups.
///
/// # Safety
///
/// `ctp` is a context; `info` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_client_info(
    ctp: *const Context,
    ioflag: c_int,
    info: *mut ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let scoid = unsafe { (*ctp).info.scoid };
    let error = ConnectClientInfo_r(scoid, info.cast(), 8);
    if error == 0 {
        // SAFETY: filled by the kernel.
        real_ids(unsafe { &mut *info }, ioflag);
    }
    error
}

/// The client's credentials, allocated, with every group if `flags`
/// asks: 0 or the error number. Free them with
/// `iofunc_client_info_ext_free()`.
///
/// # Safety
///
/// `ctp` is a context; `info` is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_client_info_ext(
    ctp: *const Context,
    ioflag: c_int,
    info: *mut *mut ClientInfo,
    flags: c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let scoid = unsafe { (*ctp).info.scoid };
    // SAFETY: as above.
    if unsafe { ConnectClientInfoExt(scoid, info.cast(), flags) } != 0 {
        return tls::get_errno();
    }
    // SAFETY: allocated and filled.
    let p = unsafe { info.read() };
    // SAFETY: as above.
    real_ids(unsafe { &mut *p }, ioflag);
    0
}

/// Frees what `iofunc_client_info_ext()` allocated.
///
/// # Safety
///
/// `info` points to null or to such credentials.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_client_info_ext_free(info: *mut *mut ClientInfo) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { ClientInfoExtFree(info.cast()) }
}

/// `iofunc_client_info_ext()`, and whether the client has each of
/// `abilities` (`ConnectClientInfoAble()`). With `O_REALIDS`, `info` is
/// required.
///
/// # Safety
///
/// As `iofunc_client_info_ext()`; `abilities` holds `nable` entries.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_client_info_able(
    ctp: *const Context,
    ioflag: c_int,
    info: *mut *mut ClientInfo,
    flags: c_int,
    abilities: *mut crate::compat::ClientAble,
    nable: c_int,
) -> c_int {
    if info.is_null() && ioflag & O_REALIDS != 0 {
        return EINVAL;
    }
    // SAFETY: the caller's guarantee.
    let scoid = unsafe { (*ctp).info.scoid };
    // SAFETY: as above.
    if unsafe { ConnectClientInfoAble(scoid, info.cast(), flags, abilities, nable) } != 0 {
        return tls::get_errno();
    }
    if !info.is_null() {
        // SAFETY: allocated and filled.
        let p = unsafe { info.read() };
        // SAFETY: as above.
        real_ids(unsafe { &mut *p }, ioflag);
    }
    0
}

/// Runs `f` with the client's credentials: `info` if given, or the
/// client's, fetched and freed.
fn with_client(
    c: *const Context,
    ioflag: c_int,
    info: *const ClientInfo,
    f: impl FnOnce(&ClientInfo) -> c_int,
) -> c_int {
    // SAFETY: the caller's info, if any, is valid.
    if let Some(i) = unsafe { info.as_ref() } {
        return f(i);
    }
    let mut p = null_mut();
    // SAFETY: c is a context.
    let error = unsafe { iofunc_client_info_ext(c, ioflag, &mut p, 1) };
    if error != 0 {
        return error;
    }
    // SAFETY: allocated and filled.
    let r = f(unsafe { &*p });
    // SAFETY: as above.
    unsafe { iofunc_client_info_ext_free(&mut p) };
    r
}

/// Whether the client may do what `check` says to the file: be its owner
/// (`S_ISUID`), be in its group (`S_ISGID`), read, write or execute
/// (`S_IREAD`, `S_IWRITE`, `S_IEXEC`). Root may do everything, except
/// execute a file nobody may execute. 0, `EPERM` or `EACCES`.
fn check_mode(attr: &IofuncAttr, check: u32, info: &ClientInfo) -> c_int {
    let euid = info.euid;
    let perm = check & (S_IREAD | S_IWRITE | S_IEXEC);
    if euid == 0 {
        let dir = attr.mode & S_IFMT == S_IFDIR;
        if perm & S_IEXEC != 0 && !dir && attr.mode & S_IXALL == 0 {
            return EACCES;
        }
        return 0;
    }
    let in_group = info.egid == attr.gid
        || info.grouplist[..(info.ngroups as usize).min(8)].contains(&attr.gid);
    let owner = euid == attr.uid;
    if check & S_ISUID != 0 && !owner {
        return EPERM;
    }
    if check & S_ISGID != 0 && !in_group {
        return EPERM;
    }
    if perm == 0 {
        return 0;
    }
    let bits = if owner {
        attr.mode
    } else if in_group {
        attr.mode << 3
    } else {
        attr.mode << 6
    };
    if perm & !bits != 0 { EACCES } else { 0 }
}

/// `check_mode()`, but an attribute's ACL decides first what a client
/// with all its groups known may read, write or execute, unless it is
/// root: `acl_hooks`' check, which says `ENOSYS` if there is no ACL.
pub(crate) fn check_access(
    ctp: *const Context,
    attr: &IofuncAttr,
    check: u32,
    info: &ClientInfo,
) -> c_int {
    let perm = check & (S_IREAD | S_IWRITE | S_IEXEC);
    let owner_checks = check & (S_ISUID | S_ISGID);
    if info.euid != 0 && perm != 0 && info.flags & crate::acl::CI_FULL_GROUPS == 0 && !ctp.is_null()
    {
        // Credentials without all the groups, as `iofunc_client_info()`
        // gives: again with them, and the caller's IDs.
        let mut p = null_mut();
        // SAFETY: the caller's context.
        let e = unsafe { iofunc_client_info_ext(ctp, 0, &mut p, 1) };
        if e != 0 {
            return e;
        }
        // SAFETY: allocated and filled.
        let full = unsafe { &mut *p };
        (full.ruid, full.euid, full.suid) = (info.ruid, info.euid, info.suid);
        (full.rgid, full.egid, full.sgid) = (info.rgid, info.egid, info.sgid);
        let r = check_access(ctp, attr, check, full);
        // SAFETY: as above.
        unsafe { iofunc_client_info_ext_free(&mut p) };
        return r;
    }
    if info.euid != 0 && perm != 0 && info.flags & crate::acl::CI_FULL_GROUPS != 0 {
        if owner_checks != 0 {
            let e = check_mode(attr, owner_checks, info);
            if e != 0 {
                return e;
            }
        }
        // SAFETY: the library's hooks, which a server may replace.
        let hook = unsafe { crate::acl::acl_hooks.check_access };
        // SAFETY: the caller's attribute and credentials.
        let e = unsafe { hook(ctp.cast_mut(), attr, perm, info) };
        if e != ENOSYS {
            return e;
        }
    }
    check_mode(attr, check, info)
}

/// `check_access()`, for C: `ENOSYS` without credentials.
///
/// # Safety
///
/// `attr` is initialised; `info` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_check_access(
    ctp: *mut Context,
    attr: *const IofuncAttr,
    check: u32,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    match unsafe { info.as_ref() } {
        // SAFETY: as above.
        Some(i) => check_access(ctp, unsafe { &*attr }, check, i),
        None => ENOSYS,
    }
}

/// Abilities (`procmgr_ability()`) are not supported yet: none is held.
#[unsafe(no_mangle)]
pub extern "C" fn iofunc_ability_check(_ctp: *const Context, _ability: c_int) -> c_int {
    0
}

/// The connect message's fields `_iofunc_open()` reads and changes.
struct OpenMsg {
    msg: *mut u8,
}

impl OpenMsg {
    /// The connect header.
    fn get(&self) -> Connect {
        hdr(self.msg)
    }
    /// Changes it.
    fn update(&self, f: impl FnOnce(&mut Connect)) {
        let mut m = self.get();
        f(&mut m);
        put(self.msg, &m);
    }
    fn ioflag(&self) -> u32 {
        self.get().ioflag
    }
    fn mode(&self) -> u32 {
        self.get().mode
    }
    fn sflag(&self) -> u16 {
        self.get().sflag
    }
    fn access(&self) -> u16 {
        self.get().access
    }
}

/// The checks of an open, and of a creation in `dattr`.
fn open_checks(
    c: &mut Context,
    m: &OpenMsg,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: Option<&ClientInfo>,
) -> c_int {
    let ioflag = (u32::from(m.access()) | !7) & m.ioflag();
    let sync = ioflag & (O_DSYNC | O_SYNC | O_RSYNC) != 0;
    // SAFETY: the caller's attributes, if any, are valid.
    let a = unsafe { attr.as_mut() };
    // SAFETY: as above.
    let d = unsafe { dattr.as_mut() };
    if let Some(d) = d {
        let mount = if d.mount.is_null() {
            a.as_ref().map_or(null_mut(), |a| a.mount)
        } else {
            d.mount
        };
        // SAFETY: the mount, if any, is valid.
        let conf = unsafe { mount.as_ref() }.map(|m| m.conf);
        if sync && conf.is_none_or(|c| c & PC_SYNC_IO == 0) {
            return EINVAL;
        }
        if ioflag & O_CREAT == 0 {
            return ENOENT;
        }
        let mut mode = m.mode() & 0xffff;
        let Some(info) = info else {
            return EINTEGRITY;
        };
        if d.mode & S_IFMT != S_IFDIR {
            return EINTEGRITY;
        }
        // SAFETY: as above.
        let mflags = unsafe { mount.as_ref() }.map_or(0, |m| m.flags);
        if d.wlocks != 0 {
            return EBUSY;
        }
        let e = check_access(core::ptr::from_ref(&*c), d, S_IWRITE, info);
        if e != 0 {
            return e;
        }
        if mflags & ST_RDONLY != 0 {
            return EROFS;
        }
        if mode & S_ISGID != 0
            && d.mode & S_ISGID != 0
            && check_access(core::ptr::from_ref(&*c), d, S_ISGID, info) != 0
        {
            mode &= !S_ISGID;
        }
        d.flags |= ATTR_MTIME | ATTR_CTIME | ATTR_DIRTY_TIME;
        mode &= !0x200;
        if mode & S_IFMT == 0 {
            mode |= S_IFREG;
        }
        m.update(|c| c.mode = mode);
        if let Some(a) = a {
            // SAFETY: a is writable.
            unsafe { iofunc_attr_init_sized(a, mode, d, info, size_of::<IofuncAttr>()) };
        }
        return 0;
    }
    let Some(a) = a else {
        return EINVAL;
    };
    if sync && mount_conf(a).is_none_or(|c| c & PC_SYNC_IO == 0) {
        return EINVAL;
    }
    if ioflag & O_NOFOLLOW != 0 && a.mode & S_IFMT == S_IFLNK {
        return ELOOP;
    }
    if ioflag & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL {
        return EEXIST;
    }
    let rd = ioflag & IO_FLAG_RD as u32 != 0;
    let wr = ioflag & IO_FLAG_WR as u32 != 0;
    if wr && a.mode & S_IFMT == S_IFDIR {
        return EISDIR;
    }
    if let Some(info) = info {
        let want = if rd { S_IREAD } else { 0 } | if wr { S_IWRITE } else { 0 };
        let e = check_access(core::ptr::from_ref(&*c), a, want, info);
        if e != 0 {
            return e;
        }
    }
    if wr && read_only(a) {
        return EROFS;
    }
    // Sharing: what others deny, and what this open denies others.
    let deny = m.sflag() & 0x70;
    if (rd && a.rlocks != 0) || (wr && a.wlocks != 0) {
        return EBUSY;
    }
    if ((deny == 0x10 || deny == 0x30) && a.rcount != 0)
        || ((deny == 0x10 || deny == 0x20) && a.wcount != 0)
    {
        return EBUSY;
    }
    if a.mode & S_IFMT == S_IFREG
        && ioflag & O_LARGEFILE == 0
        && mount_flags(a) & MOUNT_32BIT == 0
        && a.nbytes > OFF32_MAX
    {
        return EOVERFLOW;
    }
    if ioflag & O_TRUNC != 0 && wr && a.mode & S_IFMT != S_IFIFO {
        a.flags |= ATTR_MTIME | ATTR_CTIME | ATTR_DIRTY_TIME;
    }
    let _ = c;
    0
}

/// Checks an open of `attr`, or with `dattr`, a creation in that
/// directory, for the client of `info` (null: the message's client).
/// 0, or the error number.
///
/// # Safety
///
/// `ctp` is a context with a connect message `msg`; the attributes are
/// null or valid; `info` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_open(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    let m = OpenMsg { msg };
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    with_client(c, m.ioflag() as c_int, info, |i| {
        open_checks(c, &m, attr, dattr, Some(i))
    })
}

/// QNX's internal form of `iofunc_open()`, with known credentials.
///
/// # Safety
///
/// As `iofunc_open()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _iofunc_open(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    let m = OpenMsg { msg };
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    open_checks(c, &m, attr, dattr, unsafe { info.as_ref() })
}

/// The default open: checks it, under the attribute's lock, then
/// allocates an OCB and binds it.
///
/// # Safety
///
/// `ctp` is a context with a connect message `msg`; `attr` is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_open_default(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    extra: *mut c_void,
) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_open_default_cinfo(ctp, msg, attr, extra, core::ptr::null()) }
}

/// `iofunc_open_default()`, with known credentials.
///
/// # Safety
///
/// As `iofunc_open_default()`; `info` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_open_default_cinfo(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    _extra: *mut c_void,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let (lock, unlock) = lockers(unsafe { &*attr });
    let m = OpenMsg { msg };
    // SAFETY: as above.
    let c = unsafe { &mut *ctp };
    with_client(c, m.ioflag() as c_int, info, |i| {
        // SAFETY: the attribute's lock.
        let e = unsafe { lock(attr) };
        if e != 0 {
            return e;
        }
        let mut r = open_checks(c, &m, attr, null_mut(), Some(i));
        if r == 0 {
            // SAFETY: as above.
            r = unsafe { iofunc_ocb_attach(c, msg, null_mut(), attr, core::ptr::null()) };
        }
        // SAFETY: as above.
        unsafe { unlock(attr) };
        r
    })
}

/// A zeroed OCB, or null with `errno` set.
///
/// # Safety
///
/// No preconditions; unsafe as an entry of `iofunc_funcs_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_ocb_calloc(
    _ctp: *mut Context,
    _attr: *mut IofuncAttr,
) -> *mut IofuncOcb {
    let p = crate::mem::calloc::<IofuncOcb>();
    if p.is_null() {
        tls::set_errno(ENFILE);
    }
    p
}

/// Frees an OCB `iofunc_ocb_calloc()` allocated.
///
/// # Safety
///
/// `ocb` is such an OCB.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_ocb_free(ocb: *mut IofuncOcb) {
    // SAFETY: the caller's guarantee.
    unsafe { crate::mem::free(ocb.cast()) };
}

/// Fills `ocb` (allocated if null) for the open in `msg`, binds it to
/// the client's connection, and counts it in `attr`. 0, or the error
/// number.
///
/// # Safety
///
/// `ctp` is a context with a connect message `msg`; `ocb` is null or
/// writable; `attr` is valid; `funcs` is null or a table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_ocb_attach(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
    funcs: *const IoFuncs,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &mut *attr };
    if a.count == u16::MAX {
        return ENFILE;
    }
    let (calloc, free) = ocb_funcs(a);
    let mut ocb = ocb;
    let allocated = ocb.is_null();
    if allocated {
        // SAFETY: the allocator, called as iofunc_funcs_t says.
        ocb = unsafe { calloc(ctp, attr) };
        if ocb.is_null() {
            return tls::get_errno();
        }
    }
    let m = OpenMsg { msg };
    // SAFETY: allocated or the caller's.
    let o = unsafe { &mut *ocb };
    o.ioflag = ((u32::from(m.access()) | !3) & m.ioflag()) as i32;
    o.attr = attr;
    o.sflag = m.sflag();
    o.flags = 0;
    o.offset = 0;
    // SAFETY: as above.
    if unsafe { resmgr_open_bind(ctp, ocb.cast(), funcs) } == -1 {
        let e = tls::get_errno();
        if allocated {
            // SAFETY: allocated above, never bound.
            unsafe { free(ocb) };
        }
        return e;
    }
    if o.ioflag & IO_FLAG_WR != 0 {
        a.wcount += 1;
    }
    if o.ioflag & IO_FLAG_RD != 0 {
        a.rcount += 1;
    }
    if a.mode & S_IFMT != 0xc000 {
        if o.sflag & 0x50 == 0x10 {
            a.rlocks += 1;
        }
        if (o.sflag & 0x70).wrapping_sub(0x10) & 0xffe0 == 0 {
            a.wlocks += 1;
        }
    }
    a.count += 1;
    0
}

/// Uncounts `ocb` from its attribute, and stamps the times: the
/// `IOFUNC_OCB_LAST_*` flags of what it was the last of.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_ocb_detach(_ctp: *mut Context, ocb: *mut IofuncOcb) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &mut *ocb };
    // SAFETY: as above: its attribute.
    let a = unsafe { &mut *o.attr };
    // SAFETY: as above.
    unsafe { iofunc_time_update(a) };
    let mut last = 0;
    let dec = |n: &mut u16, flag: c_int, last: &mut c_int| {
        *n = n.saturating_sub(1);
        if *n == 0 {
            *last |= flag;
        }
    };
    if o.ioflag & IO_FLAG_WR != 0 {
        dec(&mut a.wcount, LAST_WRITER, &mut last);
    }
    if o.ioflag & IO_FLAG_RD != 0 {
        dec(&mut a.rcount, LAST_READER, &mut last);
    }
    if o.sflag & 0x50 == 0x10 {
        dec(&mut a.rlocks, LAST_RDLOCK, &mut last);
    }
    if (o.sflag & 0x70).wrapping_sub(0x10) & 0xffe0 == 0 {
        dec(&mut a.wlocks, LAST_WRLOCK, &mut last);
    }
    dec(&mut a.count, LAST_INUSE, &mut last);
    let _ = OCB_MMAP;
    last
}

/// Closes an OCB: detaches and frees it, under its attribute's lock.
///
/// # Safety
///
/// `ocb` is attached to `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_close_ocb(
    ctp: *mut Context,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &*attr };
    let (lock, unlock) = lockers(a);
    let (_, free) = ocb_funcs(a);
    // SAFETY: the attribute's lock.
    let e = unsafe { lock(attr) };
    if e != 0 {
        return e;
    }
    // SAFETY: as above.
    unsafe { iofunc_ocb_detach(ctp, ocb) };
    // SAFETY: the OCB's freer, for the OCB its allocator made.
    unsafe { free(ocb) };
    // SAFETY: as above.
    unsafe { unlock(attr) };
    0
}

/// The default close of an OCB.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_close_ocb_default(
    ctp: *mut Context,
    _reserved: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_close_ocb(ctp, ocb, attr) }
}

/// A connection's close (`_IO_CLOSE`): the client's record locks would
/// go; there are none yet.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_close_dup(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
    _attr: *mut IofuncAttr,
) -> c_int {
    0
}

/// The default `_IO_CLOSE`.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_close_dup_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_close_dup(ctp, msg, ocb, attr) }
}

/// Locks the OCB's attribute for one message: the framework's
/// `lock_ocb`.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_lock_ocb_default(
    _ctp: *mut Context,
    _reserved: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    let (lock, _) = lockers(unsafe { &*attr });
    // SAFETY: the attribute's lock.
    unsafe { lock(attr) }
}

/// Unlocks it: `unlock_ocb`.
///
/// # Safety
///
/// `ocb` is attached, its attribute locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_unlock_ocb_default(
    _ctp: *mut Context,
    _reserved: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    let (_, unlock) = lockers(unsafe { &*attr });
    // SAFETY: as above.
    unsafe { unlock(attr) }
}

/// Whether an I/O on `ocb` is non-blocking: `O_NONBLOCK`, unless the
/// message's extended type says otherwise. The result is `O_NONBLOCK` or 0.
fn nonblocking(ioflag: i32, xtype: u32) -> c_int {
    if xtype & XFLAG_NONBLOCK != 0 {
        O_NONBLOCK as c_int
    } else if xtype & XFLAG_BLOCK != 0 {
        0
    } else {
        ioflag & O_NONBLOCK as c_int
    }
}

/// The byte count of a read or write, with `_IO_READ64`'s high word.
fn nbytes(msg: *const u8) -> u64 {
    let m: ReadWrite = hdr(msg);
    if m.r#type == IO_READ64 || m.r#type == IO_WRITE64 {
        u64::from(m.nbytes) | u64::from(m.nbytes_hi) << 32
    } else {
        u64::from(m.nbytes)
    }
}

/// Sets it, keeping the high word for the 64-bit messages.
fn set_nbytes(msg: *mut u8, n: u64) {
    let mut m: ReadWrite = hdr(msg);
    m.nbytes = n as u32;
    if m.r#type == IO_READ64 || m.r#type == IO_WRITE64 {
        m.nbytes_hi = (n >> 32) as u32;
    }
    put(msg, &m);
}

/// The offset that follows a read's or write's header with
/// `_IO_XTYPE_OFFSET` (`struct _xtype_offset`).
fn xtype_offset(msg: *const u8) -> i64 {
    at(msg, size_of::<ReadWrite>())
}

/// Checks a read: the OCB is open for reading, a directory is read only
/// as one, the offset is valid; clamps the count to the file's limit.
/// `*nonblock` gets whether the read may block.
///
/// # Safety
///
/// `ctp` is a context with the read message `msg`; `ocb` is attached;
/// `nonblock` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_read_verify(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    nonblock: *mut c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    // SAFETY: as above.
    let o = unsafe { &*ocb };
    // SAFETY: as above: the OCB's attribute.
    let a = unsafe { &*o.attr };
    if o.ioflag & IO_FLAG_RD == 0 {
        return EBADF;
    }
    let xtype = hdr::<ReadWrite>(msg).xtype;
    let dir = a.mode & S_IFMT == S_IFDIR;
    let offset = match xtype & 0xff {
        XTYPE_OFFSET => {
            if c.size < 0x18 {
                return EBADMSG;
            }
            if dir {
                return EISDIR;
            }
            xtype_offset(msg)
        }
        XTYPE_READCOND => {
            if c.size < 0x1c {
                return EBADMSG;
            }
            if dir {
                return EISDIR;
            }
            o.offset
        }
        XTYPE_READDIR if !dir => return ENOTDIR,
        x if dir && x != XTYPE_READDIR => return EISDIR,
        _ => o.offset,
    };
    if offset < 0 {
        return EINVAL;
    }
    let n = nbytes(msg);
    if a.mode & S_IFMT == S_IFREG && n != 0 {
        let room = (max_offset(o, a) - offset).max(0) as u64;
        if room < n {
            set_nbytes(msg, room);
        }
    }
    if !nonblock.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { nonblock.write(nonblocking(o.ioflag, xtype)) };
    }
    0
}

/// Checks a write: the OCB is open for writing, the data is all in the
/// message, the file may change, the offset and size fit. Clears the
/// set-ID bits of a file written by others than root, and moves an
/// appending OCB's offset to the end.
///
/// # Safety
///
/// As `iofunc_read_verify()`, for a write message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_write_verify(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    nonblock: *mut c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    // SAFETY: as above.
    let o = unsafe { &mut *ocb };
    // SAFETY: as above.
    let a = unsafe { &mut *o.attr };
    if o.ioflag & IO_FLAG_WR == 0 {
        return EBADF;
    }
    let xtype = hdr::<ReadWrite>(msg).xtype;
    let header = match xtype & 0xff {
        XTYPE_OFFSET => size_of::<ReadWrite>() + 8,
        XTYPE_READCOND => 0x1c,
        _ => 0x10,
    };
    if c.size < header {
        return EBADMSG;
    }
    let mut n = nbytes(msg);
    let have = (c.info.srcmsglen.max(0) as usize).saturating_sub(c.offset + header);
    if (have as u64) < n {
        return EBADMSG;
    }
    if read_only(a) {
        return EROFS;
    }
    if a.mode & S_IFMT == S_IFDIR {
        return EISDIR;
    }
    let ioflag = o.ioflag as u32;
    let offset = if xtype & 0xff == XTYPE_OFFSET {
        xtype_offset(msg)
    } else if ioflag & O_APPEND != 0 {
        a.nbytes
    } else {
        o.offset
    };
    if offset < 0 {
        return EINVAL;
    }
    if a.mode & S_IFMT == S_IFREG && n != 0 {
        let max = max_offset(o, a);
        if offset >= max {
            return EFBIG;
        }
        let room = (max - offset) as u64;
        if n > room {
            n = room;
            set_nbytes(msg, n);
        }
    }
    if n != 0 && a.mode & (S_ISUID | S_ISGID | 0x200) != 0 && a.mode & S_IXALL != 0 {
        let mut info = [0u8; size_of::<ClientInfo>()];
        if ConnectClientInfo_r(c.info.scoid, info.as_mut_ptr().cast(), 0) == 0 {
            // SAFETY: filled by the kernel.
            let euid = unsafe { (*info.as_ptr().cast::<ClientInfo>()).euid };
            if euid != 0 {
                a.flags |= ATTR_DIRTY_MODE;
                a.mode &= !(S_ISUID | S_ISGID | 0x200);
            }
        }
    }
    if ioflag & O_APPEND != 0 && xtype & 0xff != XTYPE_OFFSET && n != 0 {
        o.offset = a.nbytes;
    }
    if !nonblock.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { nonblock.write(nonblocking(o.ioflag, xtype)) };
    }
    0
}

/// Checks a change of size (`_IO_SPACE`): the OCB is open for writing,
/// the range is valid and fits.
///
/// # Safety
///
/// As `iofunc_read_verify()`, for an `_IO_SPACE` message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_space_verify(
    _ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    nonblock: *mut c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &*ocb };
    // SAFETY: as above.
    let a = unsafe { &*o.attr };
    if o.ioflag & IO_FLAG_WR == 0 {
        return EBADF;
    }
    if read_only(a) {
        return EROFS;
    }
    if a.mode & S_IFMT == S_IFDIR {
        return EISDIR;
    }
    let m: io::Space = hdr(msg);
    let whence = m.whence;
    let (start, len) = (m.start as i64, m.len as i64);
    if len < 0 {
        return EINVAL;
    }
    let base = match whence {
        0 => 0,
        1 => o.offset,
        2 => a.nbytes,
        _ => return EINVAL,
    };
    let Some(at) = base.checked_add(start).filter(|&v| v >= 0) else {
        return if whence == 0 { EINVAL } else { EOVERFLOW };
    };
    if a.mode & S_IFMT == S_IFREG && at.saturating_add(len) > max_offset(o, a) {
        return EFBIG;
    }
    if !nonblock.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { nonblock.write(nonblocking(o.ioflag, 0)) };
    }
    0
}

/// Checks a sync: valid flags, and a mount that does synchronous I/O.
///
/// # Safety
///
/// `msg` is an `_IO_SYNC` message; `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_sync_verify(
    _ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    let flag = hdr::<io::Sync>(msg).flag;
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above: its attribute.
    let a = unsafe { &*attr };
    if flag & !(O_DSYNC | O_SYNC) != 0 || mount_conf(a).is_none_or(|c| c & PC_SYNC_IO == 0) {
        EINVAL
    } else {
        0
    }
}

/// The default `_IO_SYNC`.
///
/// # Safety
///
/// As `iofunc_sync_verify()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_sync_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_sync_verify(ctp, msg, ocb) }
}

/// What a sync of `ocb` with `ioflag` must write: `O_SYNC` if attributes
/// changed, `O_DSYNC` if data did, 0 for nothing.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_sync(
    _ctp: *mut Context,
    ocb: *mut IofuncOcb,
    ioflag: c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &*ocb };
    // SAFETY: as above.
    let a = unsafe { &mut *o.attr };
    if read_only(a) {
        return 0;
    }
    let flags = ioflag as u32 | o.ioflag as u32;
    if flags & O_SYNC != 0 {
        if a.flags & (ATTR_MTIME | ATTR_ATIME | ATTR_CTIME) != 0 {
            a.flags |= ATTR_DIRTY_TIME;
        }
        return if a.flags & 0xff0 != 0 {
            O_SYNC as c_int
        } else {
            0
        };
    }
    if flags & O_DSYNC != 0 && a.flags & 0xfb0 != 0 {
        return O_DSYNC as c_int;
    }
    0
}

/// The default read: checks it and replies with nothing, the end of the
/// file.
///
/// # Safety
///
/// `ctp` is a context with the read `msg`; `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_read_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let e = unsafe { iofunc_read_verify(ctp, msg, ocb, null_mut()) };
    if e != 0 {
        return e;
    }
    match hdr::<ReadWrite>(msg).xtype & 0xff {
        XTYPE_NONE | XTYPE_READDIR => {
            // SAFETY: as above.
            unsafe { (*ctp).status = 0 };
            0
        }
        _ => ENOSYS,
    }
}

/// The default write: checks it and takes the data, discarding it, as
/// `/dev/null` does.
///
/// # Safety
///
/// As `iofunc_read_default()`, for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_write_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let e = unsafe { iofunc_write_verify(ctp, msg, ocb, null_mut()) };
    if e != 0 {
        return e;
    }
    if hdr::<ReadWrite>(msg).xtype & 0xff != XTYPE_NONE {
        return ENOSYS;
    }
    let n = nbytes(msg);
    // SAFETY: as above.
    unsafe { (*ctp).status = n as c_long };
    if n != 0 {
        // SAFETY: as above.
        let attr = unsafe { (*ocb).attr };
        // SAFETY: as above.
        unsafe { (*attr).flags |= ATTR_MTIME | ATTR_CTIME };
    }
    0
}

/// Stamps the times the attribute's flags mark as changed, with the
/// current time at the mount's resolution, and marks them dirty.
///
/// # Safety
///
/// `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_time_update(attr: *mut IofuncAttr) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &mut *attr };
    if a.flags & (ATTR_MTIME | ATTR_ATIME | ATTR_CTIME) == 0 {
        return 0;
    }
    // SAFETY: the mount outlives the attribute.
    let res = unsafe { a.mount.as_ref() }
        .filter(|m| m.conf & PC_EXT != 0 && m.ext_flags & 1 != 0 && m.timeres != 0)
        .map_or(1, |m| u64::from(m.timeres));
    let now = kercall::time::_syspage_time(0);
    let now = if now == u64::MAX { 0 } else { now / res * res };
    let (s, ns) = ((now / 1_000_000_000) as i64, (now % 1_000_000_000) as u32);
    let with_ns = a.flags & ATTR_NS_TIMESTAMPS != 0;
    if a.flags & ATTR_ATIME != 0 {
        a.atime = s;
        if with_ns {
            a.atime_ns = ns;
        }
    }
    if a.flags & ATTR_MTIME != 0 {
        a.mtime = s;
        if with_ns {
            a.mtime_ns = ns;
        }
    }
    if a.flags & ATTR_CTIME != 0 {
        a.ctime = s;
        if with_ns {
            a.ctime_ns = ns;
        }
    }
    a.flags = (a.flags & !(ATTR_MTIME | ATTR_ATIME | ATTR_CTIME)) | ATTR_DIRTY_TIME;
    0
}

/// Fills `st` from the attribute, in the form `*format` asks for (0: the
/// oldest), and sets `*format` to the form filled and `*len` to its size.
///
/// # Safety
///
/// `ctp` is a context; `attr` is initialised; `st` is writable for
/// 0x78 bytes; `format` is writable; `len` is null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_stat_format(
    ctp: *mut Context,
    attr: *mut IofuncAttr,
    format: *mut c_uint,
    st: *mut u8,
    len: *mut c_uint,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &mut *attr };
    // SAFETY: as above.
    let mut form = unsafe { format.read() };
    // The common fields, then the times as the form has them.
    let mut v: QnxStat = at(st, 0);
    let with_ns = a.flags & ATTR_NS_TIMESTAMPS != 0;
    let ns = |v: u32| if with_ns { v } else { 0 };
    const MTIM: usize = offset_of!(QnxStat, st_mtim);
    let times = [
        (a.mtime, a.mtime_ns),
        (a.atime, a.atime_ns),
        (a.ctime, a.ctime_ns),
    ];
    // Form 2's 32-bit times, written over the new times' place.
    let mut short = None;
    let size = match form {
        2 => {
            let mut t = [0u32; 6];
            for (k, (s, n)) in times.iter().enumerate() {
                t[k * 2] = *s as u32;
                t[k * 2 + 1] = ns(*n);
            }
            short = Some(t);
            MTIM + size_of::<[u32; 6]>()
        }
        0 | 1 => {
            form = 1;
            v.old_times = times.map(|(s, _)| s as u32);
            MTIM
        }
        _ => {
            form = 3;
            [v.st_mtim, v.st_atim, v.st_ctim] = times.map(|(s, n)| [s, i64::from(ns(n))]);
            size_of::<QnxStat>()
        }
    };
    v.st_uid = a.uid;
    v.st_gid = a.gid;
    v.st_nlink = a.nlink;
    // SAFETY: the mount outlives the attribute.
    let mount = unsafe { a.mount.as_ref() };
    let blocksize = mount.map_or(1, |m| m.blocksize.max(1) as u32);
    let mut dev = mount.map_or(0, |m| m.dev);
    let mut ino = a.inode;
    if dev == 0 {
        dev = a.rdev;
        if a.mode & S_IFMT == S_IFNAM || dev == 0 {
            let (mut d, mut i) = (0u32, 0u64);
            // SAFETY: the caller's context.
            let id = unsafe { (*ctp).id };
            // SAFETY: d and i are writable.
            if unsafe { resmgr_devino(id, &mut d, &mut i) } != -1 {
                dev = d;
                if a.rdev == 0 && a.mode & S_IFMT != S_IFNAM {
                    a.flags |= ATTR_DIRTY_RDEV;
                    a.rdev = d;
                }
                if ino == 0 {
                    ino = i;
                    a.inode = i;
                }
            }
        }
    }
    v.st_ino = ino;
    v.st_dev = dev;
    v.st_rdev = a.rdev;
    v.st_mode = a.mode;
    v.st_blocksize = blocksize;
    let nblocks = if a.nbytes > 0 {
        (a.nbytes as u64).div_ceil(u64::from(blocksize))
    } else {
        0
    };
    v.st_nblocks = nblocks as i32;
    v.st_blksize = blocksize.max(512);
    let blocks = (a.nbytes.max(0) as u64).div_ceil(512);
    v.st_blocks = blocks as i64;
    let size64 = if a.mode & S_IFMT == S_IFBLK {
        blocks
    } else {
        a.nbytes as u64
    };
    v.st_size = size64 as i64;
    // The form's size only: a shorter form's reply ends before the rest.
    let bytes = v.as_bytes();
    // SAFETY: the caller's guarantee: st holds a struct stat.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), st, size) };
    if let Some(t) = short {
        put_at(st, MTIM, t);
    }
    // SAFETY: as above.
    unsafe { format.write(form) };
    if !len.is_null() {
        // SAFETY: as above.
        unsafe { len.write(size as u32) };
    }
    // SAFETY: as above.
    let hook = unsafe { crate::acl::acl_hooks.stat };
    // SAFETY: as above.
    unsafe { hook(ctp, attr, st) }
}

/// Fills `st` from the attribute, in the form the current message asks
/// for, and sets the context's status to it.
///
/// # Safety
///
/// As `iofunc_stat_format()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_stat(
    ctp: *mut Context,
    attr: *mut IofuncAttr,
    st: *mut u8,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let msg = c.msg.wrapping_add(c.offset);
    // SAFETY: the current message.
    let t = hdr::<Header>(msg).r#type;
    if t < IO_CONNECT {
        return ENOSYS;
    }
    let mut form = if t == IO_STAT {
        hdr::<io::Stat>(msg).format
    } else {
        0
    };
    // SAFETY: as above.
    let e = unsafe { iofunc_stat_format(ctp, attr, &mut form, st, null_mut()) };
    if e == 0 && t == IO_STAT {
        c.status = c_long::from(form as c_int);
    }
    e
}

/// The default `_IO_STAT`: the attribute, in the form asked for, in the
/// message's buffer.
///
/// # Safety
///
/// `ctp` is a context with the stat `msg`; `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_stat_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_time_update(attr) };
    let mut form = hdr::<io::Stat>(msg).format;
    let mut len = 0;
    // SAFETY: the message buffer holds a struct stat: the framework's
    // buffer is at least 0x828 bytes.
    let e = unsafe { iofunc_stat_format(ctp, attr, &mut form, msg, &mut len) };
    if e != 0 {
        return e;
    }
    let _ = (form, len);
    // Native servers speak FreeBSD's `struct stat`; `qnxd` lays it out for
    // a QNX client.
    let st: QnxStat = at(msg, 0);
    let bsd = qnxabi::stat::from_qnx(&st);
    put(msg, &bsd);
    // SAFETY: as above.
    let c = unsafe { &mut *ctp };
    c.status = c_long::from(proto::stat::FORM_FREEBSD as c_int);
    c.iov[0] = kercall::abi::Iov {
        base: msg.cast(),
        len: size_of::<proto::stat::Stat>(),
    };
    -1
}

/// Converts the `struct stat` in `st` from form `have` to form `want`:
/// its new size, or `EBADMSG` for an unknown form.
pub fn convert_stat(st: &mut [u8], have: u32, want: u32) -> Result<usize, c_int> {
    // Every form is the same up to the old 32-bit times; the forms
    // differ from the new times on.
    const OLD: usize = offset_of!(QnxStat, old_times);
    const MTIM: usize = offset_of!(QnxStat, st_mtim);
    let get32 = |s: &[u8], at: usize| u32::from_le_bytes([s[at], s[at + 1], s[at + 2], s[at + 3]]);
    let get64 =
        |s: &[u8], at: usize| u64::from_le_bytes(s[at..at + 8].try_into().unwrap_or_default());
    // The (seconds, nanoseconds) of mtime, atime, ctime.
    let times: [(u64, u64); 3] = match have {
        1 => [
            (u64::from(get32(st, OLD)), 0),
            (u64::from(get32(st, OLD + 4)), 0),
            (u64::from(get32(st, OLD + 8)), 0),
        ],
        2 => core::array::from_fn(|k| {
            (
                u64::from(get32(st, MTIM + k * 8)),
                u64::from(get32(st, MTIM + 4 + k * 8)),
            )
        }),
        3 => core::array::from_fn(|k| (get64(st, MTIM + k * 16), get64(st, MTIM + 8 + k * 16))),
        _ => return Err(EBADMSG),
    };
    match want {
        1 => {
            for (k, t) in times.iter().enumerate() {
                st[OLD + k * 4..OLD + 4 + k * 4].copy_from_slice(&(t.0 as u32).to_le_bytes());
            }
            Ok(MTIM)
        }
        2 => {
            for (k, t) in times.iter().enumerate() {
                st[MTIM + k * 8..MTIM + 4 + k * 8].copy_from_slice(&(t.0 as u32).to_le_bytes());
                st[MTIM + 4 + k * 8..MTIM + 8 + k * 8].copy_from_slice(&(t.1 as u32).to_le_bytes());
            }
            Ok(MTIM + 0x18)
        }
        3 => {
            for (k, t) in times.iter().enumerate() {
                st[MTIM + k * 16..MTIM + 8 + k * 16].copy_from_slice(&t.0.to_le_bytes());
                st[MTIM + 8 + k * 16..MTIM + 16 + k * 16].copy_from_slice(&t.1.to_le_bytes());
            }
            Ok(size_of::<QnxStat>())
        }
        _ => Err(EBADMSG),
    }
}

/// `DCMD_ALL_GETFLAGS`, `DCMD_ALL_SETFLAGS`, `DCMD_ALL_GETMOUNTFLAGS`,
/// and `DCMD_ALL_GETOWN`'s neighbour that a FIFO refuses.
const DCMD_ALL_GETFLAGS: u32 = 0x4004_0101;
const DCMD_ALL_SETFLAGS: u32 = 0x8004_0102;
const DCMD_ALL_GETMOUNTFLAGS: u32 = 0x4004_0103;
const DCMD_ALL_FADVISE: u32 = 0x8018_0106;

/// The device commands every file has: the open flags, and the mount's
/// flags. Others are `_RESMGR_DEFAULT`, for the server.
///
/// # Safety
///
/// `ctp` is a context with the devctl `msg`; `ocb` is attached to `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_devctl(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    let o = unsafe { &mut *ocb };
    // SAFETY: as above.
    let a = unsafe { &*attr };
    let dcmd = hdr::<io::Devctl>(msg).dcmd as u32;
    let head = size_of::<io::DevctlReply>();
    let room = c.msg_max_size.saturating_sub(head + c.offset);
    // The reply: the header, ret_val 0, then the value as the data.
    let reply = |c: &mut Context, v: u32| {
        let mut r: io::DevctlReply = hdr(msg);
        r.ret_val = 0;
        put(msg, &r);
        put_at(msg, head, v);
        c.iov[0] = kercall::abi::Iov {
            base: msg.cast(),
            len: head + 4,
        };
        -1
    };
    match dcmd {
        DCMD_ALL_GETFLAGS => {
            if room < 4 {
                return EMSGSIZE;
            }
            let io = o.ioflag as u32;
            let acc = if io & O_EXEC != 0 {
                O_EXEC
            } else if io & 1 != 0 {
                if io & 2 != 0 { 2 } else { 0 }
            } else if io & 2 != 0 {
                1
            } else {
                7
            };
            reply(c, (io & STATUS) | acc)
        }
        DCMD_ALL_SETFLAGS => {
            if c.size < 0x14 {
                return EMSGSIZE;
            }
            let flags: u32 = at(msg, size_of::<io::Devctl>());
            if flags & (O_DSYNC | O_SYNC | O_RSYNC) != 0
                && mount_conf(a).is_none_or(|c| c & PC_SYNC_IO == 0)
            {
                return EINVAL;
            }
            o.ioflag = ((o.ioflag as u32 & !STATUS) | (flags & STATUS)) as i32;
            0
        }
        DCMD_ALL_GETMOUNTFLAGS => {
            if room < 4 {
                return EMSGSIZE;
            }
            reply(c, mount_flags(a) & 0xff)
        }
        DCMD_ALL_FADVISE if a.mode & S_IFMT == S_IFIFO => ESPIPE,
        _ => RESMGR_DEFAULT,
    }
}

/// The default `_IO_DEVCTL`.
///
/// # Safety
///
/// As `iofunc_devctl()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_devctl_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_devctl(ctp, msg, ocb, attr) }
}

/// Checks a device command, as `requested_checks` asks: the message's
/// size (0x1), the direction against the OCB's access (0x2), and the
/// client's permissions (0x20 owner .. 0x200 execute). Deprecated.
///
/// # Safety
///
/// As `iofunc_devctl()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_devctl_verify(
    ctp: *const Context,
    msg: *const u8,
    ocb: *const IofuncOcb,
    checks: c_uint,
) -> c_int {
    if ctp.is_null() || msg.is_null() || (checks & 0x1a != 0 && ocb.is_null()) {
        return EINVAL;
    }
    // SAFETY: the caller's guarantee.
    let c = unsafe { &*ctp };
    let dcmd = hdr::<io::Devctl>(msg).dcmd as u32;
    if checks & 0x3e0 != 0 {
        let mut check = 0;
        for (bit, mode) in [
            (0x20, S_ISUID),
            (0x40, S_ISGID),
            (0x80, S_IREAD),
            (0x100, S_IWRITE),
            (0x200, S_IEXEC),
        ] {
            if checks & bit != 0 {
                check |= mode;
            }
        }
        // SAFETY: as above.
        let attr = unsafe { (*ocb).attr };
        // SAFETY: as above: its attribute.
        let a = unsafe { &*attr };
        let e = with_client(c, 0, core::ptr::null(), |i| check_access(ctp, a, check, i));
        if e != 0 {
            return e;
        }
    }
    let size = (dcmd >> 16 & 0x3fff) as isize;
    if checks & 0x400 != 0 && dcmd & 0x8000_0000 != 0 && c.info.srcmsglen < size + 0x10 {
        return EBADMSG;
    }
    if checks & 0x2 != 0 {
        // SAFETY: as above.
        let io = unsafe { (*ocb).ioflag };
        let ok = match dcmd & 0xc000_0000 {
            0x8000_0000 => io & IO_FLAG_WR != 0,
            0x4000_0000 => io & IO_FLAG_RD != 0,
            0xc000_0000 => io & 3 == 3,
            _ => true,
        };
        if !ok {
            return EPERM;
        }
    }
    0
}

/// Moves the OCB's offset, as `lseek()`: from the start, the current
/// offset or the end; replies with the new offset unless combined.
///
/// # Safety
///
/// `ctp` is a context with the lseek `msg`; `ocb` is attached to `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_lseek(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    // SAFETY: as above.
    let o = unsafe { &mut *ocb };
    // SAFETY: as above.
    let a = unsafe { &*attr };
    // SAFETY: an _IO_LSEEK message: whence at 4, offset at 8.
    let m: io::Lseek = hdr(msg);
    let (whence, sub) = (m.whence, m.combine_len);
    let delta = m.offset as i64;
    let base = match whence {
        0 => 0,
        1 => o.offset,
        // SAFETY: the OCB's attribute.
        2 => unsafe { (*o.attr).nbytes },
        _ => return EINVAL,
    };
    let Some(at) = base.checked_add(delta) else {
        return EOVERFLOW;
    };
    if at < 0 {
        return EINVAL;
    }
    if at > OFF32_MAX && max_offset(o, a) == OFF32_MAX {
        return EOVERFLOW;
    }
    o.offset = at;
    if sub & IO_COMBINE_FLAG == 0 {
        // SAFETY: the reply, the new offset, over the message.
        unsafe { msg.cast::<i64>().write_unaligned(at) };
        c.iov[0] = kercall::abi::Iov {
            base: msg.cast(),
            len: 8,
        };
        return -1;
    }
    0
}

/// The default `_IO_LSEEK`.
///
/// # Safety
///
/// As `iofunc_lseek()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_lseek_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_lseek(ctp, msg, ocb, attr) }
}

/// Changes the mode's permission bits: only the owner (or root) may, the
/// set-group-ID bit only within the group, the sticky bit only on
/// directories for others than root.
///
/// # Safety
///
/// `msg` is an `_IO_CHMOD` message; `attr` is initialised; `info` valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _iofunc_chmod(
    ctp: *mut Context,
    msg: *mut u8,
    _ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &mut *attr };
    // SAFETY: as above.
    let i = unsafe { &*info };
    let e = check_access(ctp, a, S_ISUID, i);
    if e != 0 {
        return e;
    }
    if read_only(a) {
        return EROFS;
    }
    let want = hdr::<io::Chmod>(msg).mode;
    let mut mode = (a.mode & !0xfff) | (want & 0xfff);
    if want & 0x200 != 0 && a.mode & S_IFMT != S_IFDIR && i.euid != 0 {
        mode &= !0x200;
    }
    if mode & S_ISGID != 0
        && mode & S_IFMT != S_IFDIR
        && mode & S_IXALL != 0
        && check_access(ctp, a, S_ISGID, i) != 0
    {
        mode &= !S_ISGID;
    }
    if mode != a.mode {
        a.mode = mode;
        a.flags |= ATTR_DIRTY_MODE;
    }
    // SAFETY: the caller's attribute.
    unsafe { crate::acl::chmod_update(attr) };
    a.flags |= ATTR_CTIME | ATTR_DIRTY_TIME;
    0
}

/// `_iofunc_chmod()` with the message's client.
///
/// # Safety
///
/// `ctp` is a context with the chmod `msg`; `ocb` is attached to `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_chmod(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let ioflag = unsafe { (*ocb).ioflag };
    // SAFETY: as above.
    with_client(ctp, ioflag, core::ptr::null(), |i| unsafe {
        _iofunc_chmod(ctp, msg, ocb, attr, i)
    })
}

/// The default `_IO_CHMOD`.
///
/// # Safety
///
/// As `iofunc_chmod()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_chmod_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_chmod(ctp, msg, ocb, attr) }
}

/// Changes the owner and group (-1: unchanged). With a chown-restricted
/// mount (the default), only root changes the owner, and the owner may
/// only give the file to one of their groups. Others than root clear
/// the set-ID bits.
///
/// # Safety
///
/// As `_iofunc_chmod()`, for an `_IO_CHOWN` message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _iofunc_chown(
    _ctp: *mut Context,
    msg: *mut u8,
    _ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &mut *attr };
    // SAFETY: as above.
    let i = unsafe { &*info };
    let mut m: io::Chown = hdr(msg);
    let mut gid = m.gid as u32;
    let mut uid = m.uid as u32;
    if uid == u32::MAX {
        uid = a.uid;
    }
    if gid == u32::MAX {
        gid = a.gid;
    }
    m.gid = gid as i32;
    m.uid = uid as i32;
    put(msg, &m);
    let restricted = mount_conf(a).is_none_or(|c| c & PC_CHOWN_RESTRICTED != 0);
    if i.euid != 0 {
        let in_group = i.egid == gid || i.grouplist[..(i.ngroups as usize).min(8)].contains(&gid);
        let allowed = if restricted {
            a.uid == uid && i.euid == a.uid && in_group
        } else {
            i.euid == a.uid
        };
        if !allowed && iofunc_ability_check(core::ptr::null(), 0) == 0 {
            return EPERM;
        }
    }
    if read_only(a) {
        return EROFS;
    }
    if i.euid != 0
        && a.mode & (S_ISUID | S_ISGID) != 0
        && a.mode & S_IFMT != S_IFDIR
        && a.mode & S_IXALL != 0
    {
        a.mode &= !(S_ISUID | S_ISGID);
        a.flags |= ATTR_DIRTY_MODE;
    }
    if uid != a.uid || gid != a.gid {
        a.uid = uid;
        a.gid = gid;
        a.flags |= ATTR_DIRTY_OWNER;
    }
    a.flags |= ATTR_CTIME | ATTR_DIRTY_TIME;
    0
}

/// `_iofunc_chown()` with the message's client.
///
/// # Safety
///
/// As `iofunc_chmod()`, for an `_IO_CHOWN` message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_chown(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let ioflag = unsafe { (*ocb).ioflag };
    // SAFETY: as above.
    with_client(ctp, ioflag, core::ptr::null(), |i| unsafe {
        _iofunc_chown(ctp, msg, ocb, attr, i)
    })
}

/// The default `_IO_CHOWN`.
///
/// # Safety
///
/// As `iofunc_chown()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_chown_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_chown(ctp, msg, ocb, attr) }
}

/// `UTIME_NOW` and `UTIME_OMIT`, in `_IO_UTIME64`'s nanoseconds.
const UTIME_NOW: u32 = 0x4000_0001;
const UTIME_OMIT: u32 = 0x4000_0002;

/// Sets the access and modification times: to given times only by the
/// owner, to now by anyone who may write.
///
/// # Safety
///
/// As `iofunc_chmod()`, for an `_IO_UTIME` or `_IO_UTIME64` message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_utime(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &*ocb };
    // SAFETY: as above.
    let a = unsafe { &mut *attr };
    let cur = hdr::<io::Utime>(msg).cur_flag != 0;
    let check = if cur { S_ISUID | S_IWRITE } else { S_ISUID };
    let e = with_client(ctp, o.ioflag, core::ptr::null(), |i| {
        let owner = check_access(ctp, a, S_ISUID, i);
        if owner == 0 || !cur {
            owner
        } else {
            check_access(ctp, a, check & !S_ISUID, i)
        }
    });
    if e != 0 {
        return e;
    }
    if read_only(a) {
        return EROFS;
    }
    let t = hdr::<Header>(msg).r#type;
    let with_ns = a.flags & ATTR_NS_TIMESTAMPS != 0;
    if t == IO_UTIME64 {
        let m: io::Utime64 = hdr(msg);
        for (field, (s, ns)) in [
            (m.atime_s, m.atime_ns as u32),
            (m.mtime_s, m.mtime_ns as u32),
        ]
        .into_iter()
        .enumerate()
        {
            match ns {
                UTIME_OMIT => {}
                UTIME_NOW => a.flags |= if field == 0 { ATTR_ATIME } else { ATTR_MTIME },
                n if n < 1_000_000_000 => {
                    if field == 0 {
                        a.atime = s;
                        if with_ns {
                            a.atime_ns = n;
                        }
                        a.flags &= !ATTR_ATIME;
                    } else {
                        a.mtime = s;
                        if with_ns {
                            a.mtime_ns = n;
                        }
                        a.flags &= !ATTR_MTIME;
                    }
                }
                _ => return EINVAL,
            }
        }
    } else if cur {
        a.flags |= ATTR_ATIME | ATTR_MTIME;
    } else {
        let m: io::Utime = hdr(msg);
        a.atime = i64::from(m.actime);
        a.mtime = i64::from(m.modtime);
        if with_ns {
            a.atime_ns = 0;
            a.mtime_ns = 0;
        }
        a.flags &= !(ATTR_ATIME | ATTR_MTIME);
    }
    a.flags |= ATTR_CTIME | ATTR_DIRTY_TIME;
    // SAFETY: as above.
    unsafe { iofunc_time_update(a) }
}

/// The default `_IO_UTIME`.
///
/// # Safety
///
/// As `iofunc_utime()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_utime_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_utime(ctp, msg, ocb, attr) }
}

/// The `pathconf()` values the mount's configuration gives, into the
/// context's status; `_RESMGR_DEFAULT` for others.
///
/// # Safety
///
/// `ctp` is a context with the pathconf `msg`; `attr` is initialised.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_pathconf(
    ctp: *mut Context,
    msg: *mut u8,
    _ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let a = unsafe { &*attr };
    let conf = u64::from(mount_conf(a).unwrap_or(3));
    let name = hdr::<io::Pathconf>(msg).name;
    let v: i64 = match name {
        1 => 0xffff,
        7 => (conf >> 1 & 1) as i64,
        9 => (conf & 1) as i64,
        0xc | 0xd => 1,
        0xe => (conf >> 2 & 1) as i64,
        0x12 => 0x10,
        0x13 => (conf >> 3 & 1) as i64,
        0x14 => {
            if conf & 0x40 != 0 {
                1
            } else {
                -1
            }
        }
        // SAFETY: the mount outlives the attribute.
        0x15 => unsafe { a.mount.as_ref() }.map_or(-1, |m| i64::from(m.blocksize)),
        0x16..=0x19 => -1,
        _ => return RESMGR_DEFAULT,
    };
    // SAFETY: as above.
    unsafe { pathconf_reply(ctp, msg, v) }
}

/// The reply to `_IO_PATHCONF`: the status, which libsys reads, and an
/// `i64` of reply data, which QNX's `fpathconf()` reads.
///
/// # Safety
///
/// `ctp` and `msg` are the current message's.
pub unsafe fn pathconf_reply(ctp: *mut Context, msg: *mut u8, v: i64) -> c_int {
    // SAFETY: the caller's guarantee: the message's buffer holds its 8
    // bytes, and the context is the framework's.
    unsafe {
        msg.cast::<i64>().write_unaligned(v);
        (*ctp).status = v as c_long;
        (*ctp).iov[0] = kercall::abi::Iov::raw(msg.cast(), 8);
    }
    -1
}

/// The default `_IO_PATHCONF`.
///
/// # Safety
///
/// As `iofunc_pathconf()`, with an attached `ocb`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_pathconf_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_pathconf(ctp, msg, ocb, attr) }
}

/// `_IO_OPENFD`: a new open of the OCB's file, for `openfd()`, with no
/// more access than the OCB has.
///
/// # Safety
///
/// `ctp` is a context with the openfd `msg`; `ocb` is attached to `attr`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_openfd(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &*ocb };
    let want = hdr::<io::Openfd>(msg).ioflag & (3 | O_EXEC | STATUS | O_TRUNC);
    let have = o.ioflag as u32 & (3 | O_EXEC);
    if (have ^ (3 | O_EXEC)) & want != 0 {
        return EACCES;
    }
    // A connect message with the open's flags, for the checks and the
    // new OCB.
    let mut open = [0u8; size_of::<Connect>() + 8];
    let m = OpenMsg {
        msg: open.as_mut_ptr(),
    };
    let sflag = hdr::<io::Openfd>(msg).sflag;
    m.update(|c| {
        c.ioflag = want;
        c.sflag = sflag;
        c.access = (have & 3) as u16;
    });
    // SAFETY: as above.
    let c = unsafe { &mut *ctp };
    let e = open_checks(c, &m, attr, null_mut(), None);
    if e != 0 {
        return e;
    }
    // SAFETY: the original connection's info, in the message.
    let funcs =
        unsafe { _resmgr_iofuncs(ctp, msg.wrapping_add(offset_of!(io::Openfd, info)).cast()) };
    // SAFETY: as above.
    unsafe { iofunc_ocb_attach(ctp, open.as_mut_ptr(), null_mut(), attr, funcs) }
}

/// The default `_IO_OPENFD`.
///
/// # Safety
///
/// As `iofunc_openfd()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_openfd_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    if hdr::<io::Openfd>(msg).xtype != 0 {
        return ENOSYS;
    }
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_openfd(ctp, msg, ocb, attr) }
}

/// Fills a `struct _fdinfo` from the OCB and its attribute.
///
/// # Safety
///
/// `ocb` is attached; `attr` is null or its attribute; `info` is
/// writable for 64 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_fdinfo(
    _ctp: *mut Context,
    ocb: *mut IofuncOcb,
    attr: *mut IofuncAttr,
    info: *mut u8,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let o = unsafe { &*ocb };
    let attr = if attr.is_null() { o.attr } else { attr };
    // SAFETY: as above.
    let a = unsafe { &*attr };
    let flags = u32::from(!a.lock_list.is_null() && a.lock_list as usize > 1)
        | if a.mmap_list.is_null() { 0 } else { 2 };
    let fd = proto::info::Fdinfo {
        mode: a.mode,
        ioflag: o.ioflag as u32,
        offset: o.offset as u64,
        size: a.nbytes as u64,
        flags,
        sflag: o.sflag,
        count: a.count,
        rcount: a.rcount,
        wcount: a.wcount,
        rlocks: a.rlocks,
        wlocks: a.wlocks,
        ..Default::default()
    };
    put(info, &fd);
    0
}

/// The default `_IO_FDINFO`: the OCB's `struct _fdinfo`, and unless the
/// flags say otherwise, the attachment's path.
///
/// # Safety
///
/// `ctp` is a context with the fdinfo `msg`; `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_fdinfo_default(
    ctp: *mut Context,
    msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let Some(room) = c.msg_max_size.checked_sub(0x48 + c.offset) else {
        return EMSGSIZE;
    };
    let req: io::FdinfoReq = hdr(msg);
    let flags = req.flags;
    let maxbuf = (req.path_len as usize).min(room);
    // The reply's header, two zero words, before the struct _fdinfo.
    put(msg, &[0u32; 2]);
    // SAFETY: the reply's struct _fdinfo, after its 8-byte header.
    unsafe { iofunc_fdinfo(ctp, ocb, null_mut(), msg.wrapping_add(8)) };
    let mut n = 0;
    if flags & 2 == 0 {
        // SAFETY: the path, after the reply's 0x48 bytes, within the buffer.
        let r =
            unsafe { resmgr_pathname(c.id, flags, msg.wrapping_add(0x48).cast(), maxbuf as c_int) };
        if r != -1 {
            n = r;
        }
    }
    c.status = c_long::from(n);
    c.iov[0] = kercall::abi::Iov {
        base: msg.cast(),
        len: 0x48 + (n as usize).min(maxbuf),
    };
    -1
}

/// A client wants to be unblocked: a record lock would be waited for; no
/// locks yet, so the server decides.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_unblock(_ctp: *mut Context, _attr: *mut IofuncAttr) -> c_int {
    RESMGR_DEFAULT
}

/// The default unblock.
///
/// # Safety
///
/// `ocb` is attached.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_unblock_default(
    ctp: *mut Context,
    _msg: *mut u8,
    ocb: *mut IofuncOcb,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let attr = unsafe { (*ocb).attr };
    // SAFETY: as above.
    unsafe { iofunc_unblock(ctp, attr) }
}

/// Record locks (`fcntl()`'s `F_SETLK`), not supported yet.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_lock_default(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
) -> c_int {
    ENOSYS
}

/// Mapping a server's file, not supported yet.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mmap_default(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
) -> c_int {
    ENOSYS
}

/// As `iofunc_mmap_default()`.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mmap_ext(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
    _attr: *mut IofuncAttr,
    _flags: u32,
    _st: *const u8,
) -> c_int {
    ENOSYS
}

/// `iofunc_lock_default()`, `iofunc_mmap_default()` and
/// `iofunc_power_default()` with the attribute given: not supported yet
/// either.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_lock(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
    _attr: *mut IofuncAttr,
) -> c_int {
    ENOSYS
}

/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mmap(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
    _attr: *mut IofuncAttr,
) -> c_int {
    ENOSYS
}

/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mmap_default_ext(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
) -> c_int {
    ENOSYS
}

/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_power(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
    _attr: *mut IofuncAttr,
) -> c_int {
    ENOSYS
}

/// Power management, not supported.
///
/// # Safety
///
/// No preconditions beyond the framework's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_power_default(
    _ctp: *mut Context,
    _msg: *mut u8,
    _ocb: *mut IofuncOcb,
) -> c_int {
    ENOSYS
}

/// Checks an unlink of `attr` from the directory `dattr`, and drops a
/// link: the caller may write the directory (or, sticky, owns the file
/// or the directory); a directory is removed only empty.
///
/// # Safety
///
/// `ctp` is a context; `msg` is null or an unlink connect message; the
/// attributes are null or valid; `info` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_unlink(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let Some(a) = (unsafe { attr.as_mut() }) else {
        return EINTEGRITY;
    };
    // SAFETY: as above.
    let d = unsafe { dattr.as_mut() };
    if d.as_ref().is_some_and(|d| d.mode & S_IFMT != S_IFDIR) {
        return EINTEGRITY;
    }
    let ioflag = if msg.is_null() {
        0
    } else {
        hdr::<Connect>(msg).ioflag as c_int
    };
    let e = with_client(ctp, ioflag, info, |i| {
        if !msg.is_null() {
            let want_dir = hdr::<Connect>(msg).mode & S_IFMT == S_IFDIR;
            let is_dir = a.mode & S_IFMT == S_IFDIR;
            if want_dir && !is_dir {
                return ENOTDIR;
            }
            if want_dir && a.nlink > 2 {
                return ENOTEMPTY;
            }
            if is_dir && !want_dir && check_access(ctp, a, 0, i) != 0 {
                return EPERM;
            }
        }
        match &d {
            Some(d) => {
                let e = check_access(ctp, d, S_IWRITE, i);
                if e != 0 {
                    return e;
                }
                if d.mode & 0x200 != 0
                    && check_access(ctp, a, S_ISUID, i) != 0
                    && check_access(ctp, d, S_ISUID, i) != 0
                {
                    return EPERM;
                }
                0
            }
            None => {
                if check_access(ctp, a, S_IWRITE, i) == 0 || check_access(ctp, a, S_ISUID, i) == 0 {
                    0
                } else {
                    EPERM
                }
            }
        }
    });
    if e != 0 {
        return e;
    }
    if read_only(a) {
        return EROFS;
    }
    if a.mode & S_IFMT == S_IFDIR {
        if a.nlink < 2 {
            return EINTEGRITY;
        }
        a.nlink -= 2;
        if let Some(d) = d {
            d.nlink = d.nlink.saturating_sub(1);
            d.flags |= ATTR_DIRTY_NLINK | ATTR_MTIME | ATTR_CTIME | ATTR_DIRTY_TIME;
        }
    } else {
        if a.nlink == 0 {
            return EINTEGRITY;
        }
        a.nlink -= 1;
        if let Some(d) = d {
            d.flags |= ATTR_MTIME | ATTR_CTIME | ATTR_DIRTY_TIME;
        }
    }
    a.flags |= ATTR_DIRTY_NLINK | ATTR_CTIME | ATTR_DIRTY_TIME;
    0
}

/// Checks a creation (mknod) in `dattr`, and initialises `attr`.
///
/// # Safety
///
/// As `iofunc_unlink()`, with a mknod message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_mknod(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    let m = OpenMsg { msg };
    m.update(|c| c.ioflag |= O_CREAT);
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    with_client(c, m.ioflag() as c_int, info, |i| {
        open_checks(c, &m, attr, dattr, Some(i))
    })
}

/// Checks a new link to `attr` in `dattr`, and counts it.
///
/// # Safety
///
/// As `iofunc_unlink()`, with a link message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_link(
    ctp: *mut Context,
    msg: *mut u8,
    attr: *mut IofuncAttr,
    dattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let (Some(a), Some(d)) = (unsafe { attr.as_mut() }, unsafe { dattr.as_mut() }) else {
        return EINTEGRITY;
    };
    let ioflag = hdr::<Connect>(msg).ioflag as c_int;
    let e = with_client(ctp, ioflag, info, |i| {
        check_access(ctp, d, S_IWRITE | S_IEXEC, i)
    });
    if e != 0 {
        return e;
    }
    if a.mount != d.mount {
        return EXDEV;
    }
    if read_only(d) {
        return EROFS;
    }
    a.nlink += 1;
    a.flags |= ATTR_DIRTY_NLINK | ATTR_CTIME | ATTR_DIRTY_TIME;
    d.flags |= ATTR_MTIME | ATTR_CTIME | ATTR_DIRTY_TIME;
    0
}

/// Checks a rename: the old name may be unlinked, the new one created.
///
/// # Safety
///
/// As `iofunc_unlink()`, with a rename message.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_rename(
    ctp: *mut Context,
    msg: *mut u8,
    oldattr: *mut IofuncAttr,
    olddattr: *mut IofuncAttr,
    newattr: *mut IofuncAttr,
    newdattr: *mut IofuncAttr,
    info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let (Some(o), Some(od), Some(nd)) = (
        unsafe { oldattr.as_ref() },
        unsafe { olddattr.as_ref() },
        unsafe { newdattr.as_ref() },
    ) else {
        return EINTEGRITY;
    };
    let ioflag = hdr::<Connect>(msg).ioflag as c_int;
    let e = with_client(ctp, ioflag, info, |i| {
        let e = check_access(ctp, od, S_IWRITE, i);
        if e != 0 {
            return e;
        }
        let e = check_access(ctp, nd, S_IWRITE, i);
        if e != 0 {
            return e;
        }
        // SAFETY: as above.
        if let Some(n) = unsafe { newattr.as_ref() }
            && (n.mode & S_IFMT == S_IFDIR) != (o.mode & S_IFMT == S_IFDIR)
        {
            return if n.mode & S_IFMT == S_IFDIR {
                EISDIR
            } else {
                ENOTDIR
            };
        }
        0
    });
    if e != 0 {
        return e;
    }
    if od.mount != nd.mount {
        return EXDEV;
    }
    if read_only(nd) { EROFS } else { 0 }
}

/// Checks a readlink: the attribute is a symbolic link.
///
/// # Safety
///
/// `attr` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_readlink(
    _ctp: *mut Context,
    _msg: *mut u8,
    attr: *mut IofuncAttr,
    _info: *const ClientInfo,
) -> c_int {
    // SAFETY: the caller's guarantee.
    match unsafe { attr.as_ref() } {
        Some(a) if a.mode & S_IFMT == S_IFLNK => 0,
        Some(_) => EINVAL,
        None => EINTEGRITY,
    }
}

/// `_NOTIFY_ACTION_*`.
const ACTION_TRANARM: c_int = 0;
const ACTION_CONDARM: c_int = 1;
const ACTION_POLL: c_int = 2;
const ACTION_POLLARM: c_int = 3;
const ACTION_EDGEARM: c_int = 4;
/// `_NOTIFY_COND_EXTEN`: the conditions are extended ones, in the low
/// bits; `_NOTIFY_COND_INPUT` and the others are in the top nibble.
const COND_EXTEN: u32 = 0x8000_0000;
const COND_SHIFT: u32 = 28;

/// The default counts: one byte or one slot satisfies a condition.
static ONES: [c_int; 16] = [1; 16];

/// The conditions in `flags`, as indexes of `nop`: the top nibble's
/// input, output and out-of-band, and with `_NOTIFY_COND_EXTEN`, the
/// extended ones from index 3.
fn conditions(flags: u32) -> u32 {
    let basic = flags >> COND_SHIFT & 7;
    if flags & COND_EXTEN != 0 {
        basic | (flags & 0x7fff)
    } else {
        basic
    }
}

/// `ionotify()`'s request: replies with the conditions of `trig` that
/// the client asked about, and arms the client's event on the others as
/// the action says. `notifycnts` are the counts that satisfy each
/// condition; `*armed` (if not null) the number of `nop` entries on
/// entry, and whether an event was armed on return.
///
/// # Safety
///
/// `ctp` is a context with the notify `msg`; `nop` holds the entries;
/// `notifycnts` is null or holds as many counts; `armed` is null or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_notify(
    ctp: *mut Context,
    msg: *mut u8,
    nop: *mut Notify,
    trig: c_int,
    notifycnts: *const c_int,
    armed: *mut c_int,
) -> c_int {
    // SAFETY: the caller's guarantee.
    let c = unsafe { &mut *ctp };
    let mut entries = 3usize;
    let mut dummy = 0;
    // SAFETY: as above.
    let armed = match unsafe { armed.as_mut() } {
        Some(a) => {
            if trig < 0 {
                entries = (*a).clamp(0, 15) as usize;
            }
            a
        }
        None => &mut dummy,
    };
    *armed = 0;
    let request: io::Notify = hdr(msg);
    let action = request.action;
    let flags = request.flags;
    let exten = flags & COND_EXTEN;
    let trig = trig as u32;
    let have = conditions(trig | trig << COND_SHIFT) & conditions(flags);
    let wanted = conditions(flags);
    // Every action visits the client's own entry on each condition asked
    // about: an arming one arms it, the others disarm it, as QNX's.
    let arm = match action {
        ACTION_POLL => false,
        ACTION_POLLARM | ACTION_CONDARM => have == 0,
        ACTION_TRANARM | ACTION_EDGEARM => true,
        _ => return EINVAL,
    };
    if action == ACTION_TRANARM {
        // Only one client may arm for transitions, with no event pending.
        // SAFETY: the caller's guarantee: the entries.
        let mut e = unsafe { (*nop).list };
        while !e.is_null() {
            // SAFETY: an event of the list.
            let ev = unsafe { &*e };
            if ev.event[0] as u8 != 0
                && (request.event.notify as u8 != 0 || ev.scoid != c.info.scoid)
            {
                return EBUSY;
            }
            e = ev.next;
        }
    }
    // SAFETY: as above: the entries.
    let nops = unsafe { core::slice::from_raw_parts_mut(nop, entries) };
    let event: [u64; 4] = at(msg, offset_of!(io::Notify, event));
    for (k, n) in nops.iter_mut().enumerate() {
        if wanted & (1 << k) == 0 {
            continue;
        }
        // This client's entry on the list, or a new one.
        let mut e = n.list;
        while !e.is_null() {
            // SAFETY: an event of the list.
            let ev = unsafe { &*e };
            if ev.rcvid as c_int == c.rcvid as c_int && ev.coid == c.info.coid {
                break;
            }
            e = ev.next;
        }
        if e.is_null() {
            e = crate::mem::calloc::<NotifyEvent>();
            if e.is_null() {
                return ENOMEM;
            }
            // SAFETY: allocated above.
            unsafe { (*e).next = n.list };
            n.list = e;
        }
        // SAFETY: as above.
        let ev = unsafe { &mut *e };
        ev.rcvid = c.rcvid;
        ev.scoid = c.info.scoid;
        ev.coid = c.info.coid;
        ev.flags = (ev.flags & 0x7fff_ffff) | exten;
        if !arm {
            ev.cnt = c_int::MAX;
            ev.event[0] &= !0xffff_ffff;
            continue;
        }
        ev.event = event;
        ev.cnt = if action == ACTION_TRANARM {
            0
        } else if notifycnts.is_null() {
            ONES[k]
        } else {
            // SAFETY: as above.
            unsafe { notifycnts.wrapping_add(k).read() }
        };
        n.cnt = n.cnt.min(ev.cnt);
        *armed = 1;
    }
    if action == ACTION_CONDARM && have == 0 {
        return EAGAIN;
    }
    // A transition arm reports nothing.
    let reply = if have == 0 || action == ACTION_TRANARM {
        0
    } else {
        exten | (have & 7) << COND_SHIFT | if exten != 0 { have & 0x7fff } else { 0 }
    };
    // The conditions that hold go in the reply's flags, where the
    // request's action was (`struct _io_notify_reply`), as QNX's library
    // puts them.
    let mut answer = request;
    answer.action = reply as i32;
    put(msg, &answer);
    c.iov[0] = kercall::abi::Iov {
        base: msg.cast(),
        len: size_of::<io::Notify>(),
    };
    -1
}

/// `index`'s bits that are not the condition: the entry is the rest.
const NOTIFY_INDEX_FLAGS: c_int = 0x1060;
/// `SI_NOTIFY`, the code of `poll()`'s and `select()`'s signals.
const SI_NOTIFY: i16 = -6;

/// Delivers the events armed on condition `index` whose count `cnt`
/// satisfies, and disarms them.
///
/// # Safety
///
/// `nop` holds the entry `index` names.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_notify_trigger(nop: *mut Notify, cnt: c_int, index: c_int) {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_notify_trigger_strict(null_mut(), nop, cnt, index) }
}

/// `iofunc_notify_trigger()`, for the events of the current message's
/// connection only if `ctp` is not null. `index` is the condition, which
/// names the entry of `nop`, with flags (`0x1060`) beside it. An event of
/// `poll()`'s (a signal with code `SI_NOTIFY`) gets the condition in its
/// value; and, if the client asked for the extended conditions, in its
/// code too, negated, as `poll()` reads them.
///
/// # Safety
///
/// As `iofunc_notify_trigger()`; `ctp` is null or a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_notify_trigger_strict(
    ctp: *mut Context,
    nop: *mut Notify,
    cnt: c_int,
    index: c_int,
) {
    let slot = (index & !NOTIFY_INDEX_FLAGS) as usize;
    let bit = 1u32 << (index & 0x1f);
    // SAFETY: the caller's guarantee.
    let n = unsafe { &mut *nop.wrapping_add(slot) };
    // SAFETY: as above.
    let only = unsafe { ctp.as_ref() }.map(|c| (c.info.scoid, c.info.coid));
    n.cnt = c_int::MAX;
    let mut e = n.list;
    while !e.is_null() {
        // SAFETY: an event of the list.
        let ev = unsafe { &mut *e };
        e = ev.next;
        let flags = ev.flags & !1;
        ev.flags = flags;
        let kind = ev.event[0] as u8;
        if kind == 0 {
            continue;
        }
        let mine = only.is_none_or(|(s, co)| ev.scoid == s && ev.coid == co);
        if !mine || cnt < ev.cnt {
            n.cnt = n.cnt.min(ev.cnt);
            continue;
        }
        // The event's value (its low word) and code, in struct sigevent.
        let code = ev.event[3] as u16 as i16;
        if (1..=4).contains(&kind) && code == SI_NOTIFY {
            let mut value = ev.event[2] as u32 | 0x1000_0000 << (index & 0x1f);
            let mut code = code;
            if (flags as c_int) < 0 {
                value |= 0x8000_0000;
                code = (bit as i16 | (index & NOTIFY_INDEX_FLAGS) as i16).wrapping_neg();
            }
            ev.event[2] = (ev.event[2] & !0xffff_ffff) | u64::from(value);
            ev.event[3] = (ev.event[3] & !0xffff) | u64::from(code as u16);
        }
        let _ = MsgDeliverEvent_r(ev.rcvid, ev.event.as_ptr().cast());
        // Disarmed: the count, and the event's type.
        ev.cnt = c_int::MAX;
        ev.event[0] &= !0xffff_ffff;
    }
}

/// Removes the current message's connection's events from the first
/// `lim` entries (all connections' if `ctp` is null).
///
/// # Safety
///
/// `nop` holds `lim` entries; `ctp` is null or a context.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_notify_remove_strict(
    ctp: *mut Context,
    nop: *mut Notify,
    lim: c_int,
) {
    // SAFETY: the caller's guarantee.
    let only = unsafe { ctp.as_ref() }.map(|c| (c.info.scoid, c.info.coid));
    for k in 0..lim.max(0) as usize {
        // SAFETY: as above.
        let n = unsafe { &mut *nop.wrapping_add(k) };
        let mut link = &raw mut n.list;
        loop {
            // SAFETY: the list's head or an event's next.
            let e = unsafe { link.read() };
            if e.is_null() {
                break;
            }
            // SAFETY: an event of the list.
            let ev = unsafe { &mut *e };
            if only.is_none_or(|(s, co)| ev.scoid == s && ev.coid == co) {
                // SAFETY: as above.
                unsafe { link.write(ev.next) };
                // SAFETY: allocated by iofunc_notify().
                unsafe { crate::mem::free(e.cast()) };
            } else {
                link = &raw mut ev.next;
            }
        }
    }
}

/// Removes the current connection's events from the three basic entries.
///
/// # Safety
///
/// As `iofunc_notify_remove_strict()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_notify_remove(ctp: *mut Context, nop: *mut Notify) {
    // SAFETY: the caller's guarantee.
    unsafe { iofunc_notify_remove_strict(ctp, nop, 3) }
}

/// `struct _iofunc_track_funcs`: how a server finds an attribute's data
/// quickly, and how it frees it.
#[repr(C)]
pub struct TrackFuncs {
    handle: Option<unsafe extern "C" fn(*const IofuncAttr, *mut *mut c_void) -> c_int>,
    cleanup: Option<unsafe extern "C" fn(*mut IofuncAttr, *mut c_void)>,
}

/// One attribute's data in a tracker, by its inode and device.
struct TrackEntry {
    next: *mut TrackEntry,
    inode: u64,
    dev: u32,
    data: *mut c_void,
}

/// `iofunc_track_t`: a server's data for attributes, kept by the library.
pub struct Track {
    next: *mut Track,
    funcs: *const TrackFuncs,
    lock: crate::compat::Mutex,
    entries: *mut TrackEntry,
}

/// The trackers, one for each set of functions.
static mut TRACKS: *mut Track = null_mut();
static TRACKS_LOCK: crate::compat::Mutex = crate::compat::Mutex::new();

/// `attr`'s inode and device.
fn key(attr: &IofuncAttr) -> (u64, u32) {
    // SAFETY: an attribute's mount is null or a mount.
    (
        attr.inode,
        unsafe { attr.mount.as_ref() }.map_or(0, |m| m.dev),
    )
}

/// The tracker of `funcs`, made the first time: null without memory.
#[unsafe(no_mangle)]
pub extern "C" fn iofunc_track_init(funcs: *const TrackFuncs) -> *mut Track {
    let _g = TRACKS_LOCK.lock();
    let head = &raw mut TRACKS;
    // SAFETY: the list, under its lock.
    let mut t = unsafe { *head };
    // SAFETY: the list's trackers.
    while let Some(tr) = unsafe { t.as_ref() } {
        if tr.funcs == funcs {
            return t;
        }
        t = tr.next;
    }
    let new = crate::mem::calloc::<Track>();
    if !new.is_null() {
        // SAFETY: the list, under its lock.
        let next = unsafe { *head };
        let t = Track {
            next,
            funcs,
            lock: crate::compat::Mutex::new(),
            entries: null_mut(),
        };
        // SAFETY: room for a tracker.
        unsafe { new.write(t) };
        // SAFETY: the list, under its lock.
        unsafe { *head = new };
    }
    new
}

/// The entry of `attr` in `trk`, made if `create`.
///
/// # Safety
///
/// `trk` is a tracker, its lock held.
unsafe fn entry(entries: &mut *mut TrackEntry, attr: &IofuncAttr, create: bool) -> *mut TrackEntry {
    let (inode, dev) = key(attr);
    let mut e = *entries;
    // SAFETY: the tracker's entries.
    while let Some(en) = unsafe { e.as_ref() } {
        if en.inode == inode && en.dev == dev {
            return e;
        }
        e = en.next;
    }
    if !create {
        return null_mut();
    }
    let new = crate::mem::calloc::<TrackEntry>();
    if !new.is_null() {
        // SAFETY: room for an entry.
        unsafe {
            new.write(TrackEntry {
                next: *entries,
                inode,
                dev,
                data: null_mut(),
            })
        };
        *entries = new;
    }
    new
}

/// The data `trk` keeps for `attr` (made, empty, if it keeps none), its
/// place into `*pdata`: the entry, which the server's `handle` function
/// may give back; null without memory.
///
/// # Safety
///
/// `trk` is a tracker; `attr` an attribute; `pdata` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_track_data(
    trk: *mut Track,
    attr: *const IofuncAttr,
    pdata: *mut *mut *mut c_void,
) -> *mut c_void {
    // SAFETY: the caller's guarantee.
    let t = unsafe { &mut *trk };
    let _g = t.lock.lock();
    // SAFETY: as above.
    let a = unsafe { &*attr };
    // SAFETY: as above, under the lock.
    let e = unsafe { entry(&mut t.entries, a, true) };
    // SAFETY: the new or found entry, or null.
    if let Some(en) = unsafe { e.as_mut() } {
        // SAFETY: as above.
        unsafe { pdata.write(&raw mut en.data) };
    }
    e.cast()
}

/// The data `trk` keeps for `attr`, or null: through the server's
/// `handle` function if it knows the entry.
///
/// # Safety
///
/// As `iofunc_track_data()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_track_get(trk: *mut Track, attr: *const IofuncAttr) -> *mut c_void {
    // SAFETY: the caller's guarantee.
    let t = unsafe { &mut *trk };
    let _g = t.lock.lock();
    // SAFETY: as above.
    let a = unsafe { &*attr };
    let mut vp = null_mut();
    // SAFETY: the tracker's functions; the server's function.
    let quick = unsafe { (*t.funcs).handle }.is_some_and(|h| unsafe { h(attr, &mut vp) } == 0);
    // SAFETY: an entry the server was given, or null.
    let given =
        unsafe { vp.cast::<TrackEntry>().as_ref() }.filter(|e| quick && (e.inode, e.dev) == key(a));
    let e = match given {
        Some(e) => core::ptr::from_ref(e).cast_mut(),
        // SAFETY: as above, under the lock.
        _ => unsafe { entry(&mut t.entries, a, false) },
    };
    // SAFETY: an entry, or null.
    unsafe { e.as_ref() }.map_or(null_mut(), |e| e.data)
}

/// Lets go of `attr`'s data in `trk` (in every tracker for null) when the
/// attribute is gone, or at once with `destroy`: the server's `cleanup`
/// frees it.
///
/// # Safety
///
/// `trk` is null or a tracker; `attr` an attribute.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn iofunc_track_close(
    trk: *mut Track,
    attr: *mut IofuncAttr,
    destroy: c_int,
) {
    let (mut t, destroy, one) = if trk.is_null() {
        // SAFETY: the list's head.
        let head = &raw const TRACKS;
        // SAFETY: the list's head.
        (unsafe { *head }, false, false)
    } else {
        (trk, destroy != 0, true)
    };
    // SAFETY: the caller's guarantee.
    let a = unsafe { &*attr };
    // SAFETY: the trackers.
    while let Some(tr) = unsafe { t.as_mut() } {
        if destroy || (a.count == 0 && a.nlink == 0) {
            let g = tr.lock.lock();
            let key = key(a);
            let mut link = &raw mut tr.entries;
            loop {
                // SAFETY: the tracker's entries, under its lock.
                let cur = unsafe { *link };
                // SAFETY: as above.
                let Some(e) = (unsafe { cur.as_mut() }) else {
                    break;
                };
                if (e.inode, e.dev) != key {
                    link = &raw mut e.next;
                    continue;
                }
                // SAFETY: as above.
                unsafe { *link = e.next };
                drop(g);
                if !e.data.is_null()
                    // SAFETY: the tracker's functions.
                    && let Some(cleanup) = unsafe { (*tr.funcs).cleanup }
                {
                    // SAFETY: the server's function, with its data.
                    unsafe { cleanup(attr, e.data) };
                }
                // SAFETY: from calloc().
                unsafe { crate::mem::free(core::ptr::from_mut(e).cast()) };
                break;
            }
        }
        if one {
            break;
        }
        t = tr.next;
    }
}

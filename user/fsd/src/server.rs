//! The resource manager: one dispatch, any number of mounts.

use core::ffi::{c_int, c_long, c_uint, c_void};
use core::ptr::null;
use std::ffi::CString;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};

use kercall::neutrino::connect::ConnectClientInfo;
use kercall::neutrino::msg::{MsgRead, MsgReply};
use proto::errno as e;
use proto::fcntl::{O_APPEND, O_CREAT, O_DIRECTORY, O_EXCL, O_TRUNC};
use proto::connect::Connect;
use proto::info::ClientInfo;
use proto::io;
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_NOREPLY, ResmgrAttr};
use resmgr::dispatch::{Dispatch, dispatch_block, dispatch_context_alloc, dispatch_create, dispatch_handler};
use resmgr::iofunc::{
    iofunc_attr_init, iofunc_close_ocb_default, iofunc_devctl_default, iofunc_fdinfo, iofunc_func_init,
    iofunc_ocb_attach, iofunc_stat_default,
};
use resmgr::mem::calloc_bytes;
use resmgr::resmgr::{resmgr_attach, resmgr_detach};

use crate::{FileSystem, Node, Result, S_IFDIR, S_IFMT, S_IFREG, S_IFSOCK, S_ISVTX, Who};

/// `diskd`'s commands for a mount, and QNX's statvfs.
const DCMD_UNMOUNT: u32 = 0x4d02;
const DCMD_NAMES: u32 = 0x4d03;
const DCMD_STATVFS: u32 = 0x020d;
/// `IOFUNC_ATTR_ATIME`, `_MTIME` and `_CTIME`.
const ATTR_TIMES: u32 = 0x1 | 0x2 | 0x4;
const XTYPE_READDIR: u32 = 9;
const XTYPE_MASK: u32 = 0xff;
/// The open's access, in `ioflag` (`O_ACCMODE` plus one).
const IO_FLAG_RD: u32 = 1;
const IO_FLAG_WR: u32 = 2;
/// `_IO_` message indices in the function table.
const F_READ: usize = 0;
const F_WRITE: usize = 1;
const F_CLOSE_OCB: usize = 2;
const F_STAT: usize = 3;
const F_DEVCTL: usize = 5;
const F_CHMOD: usize = 9;
const F_CHOWN: usize = 10;
const F_UTIME: usize = 11;
const F_SPACE: usize = 15;
const F_FDINFO: usize = 0x0e;
const F_SYNC: usize = 0x18;
const F_READ64: usize = 0x1d;
const F_WRITE64: usize = 0x1e;
const F_UTIME64: usize = 0x20;
/// Connect functions.
const C_OPEN: usize = 0;
const C_UNLINK: usize = 1;
const C_RENAME: usize = 2;
const C_MKNOD: usize = 3;
const F_GROWSP64: u16 = 0x6d;
const F_FREESP64: u16 = 0x6f;
const MOUNT_TABLE: &str = "/var/run/mounts";

/// A path served: its file system, and the attribute the path is attached
/// with (the handle every connect message brings).
struct Mount {
    fs: Mutex<Box<dyn FileSystem>>,
    root: usize,
    path: String,
    source: String,
    id: AtomicI32,
    /// Files open on it, an `unmount()`'s own included.
    open: AtomicI32,
}

static MOUNTS: Mutex<Vec<Option<Arc<Mount>>>> = Mutex::new(Vec::new());
static DISPATCH: AtomicUsize = AtomicUsize::new(0);
static INIT: Once = Once::new();
/// What `umount()` of a mount does; `EBUSY` without.
static UNMOUNT: Mutex<Option<fn(usize, bool) -> Result<()>>> = Mutex::new(None);

// SAFETY: all zeroes is a valid start for these; `init` fills them.
static mut CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut IO: IoFuncs = unsafe { core::mem::zeroed() };

/// An open file: the framework's OCB and attribute, its path and mount.
#[repr(C)]
struct Ocb {
    ocb: IofuncOcb,
    attr: *mut IofuncAttr,
    path: *mut String,
    mount: usize,
}

fn init() {
    INIT.call_once(|| {
        // SAFETY: the framework's functions, with statics of this program,
        // once, before any message.
        unsafe {
            DISPATCH.store(dispatch_create() as usize, Ordering::Relaxed);
            iofunc_func_init(9, &raw mut CONNECT, 33, &raw mut IO);
            let c = &mut *(&raw mut CONNECT);
            c.funcs[C_OPEN] = Some(io_open);
            c.funcs[C_UNLINK] = Some(io_unlink);
            c.funcs[C_RENAME] = Some(io_rename);
            c.funcs[C_MKNOD] = Some(io_mknod);
            let io = &mut *(&raw mut IO);
            io.funcs[F_READ] = Some(io_read);
            io.funcs[F_READ64] = Some(io_read);
            io.funcs[F_WRITE] = Some(io_write);
            io.funcs[F_WRITE64] = Some(io_write);
            io.funcs[F_STAT] = Some(io_stat);
            io.funcs[F_SPACE] = Some(io_space);
            io.funcs[F_SYNC] = Some(io_sync);
            io.funcs[F_CLOSE_OCB] = Some(io_close_ocb);
            io.funcs[F_DEVCTL] = Some(io_devctl);
            io.funcs[F_CHMOD] = Some(io_chmod);
            io.funcs[F_CHOWN] = Some(io_chown);
            io.funcs[F_UTIME] = Some(io_utime);
            io.funcs[F_UTIME64] = Some(io_utime);
            io.funcs[F_FDINFO] = Some(io_fdinfo);
        }
    });
}

/// The dispatch every attachment of this process shares: a server's own
/// paths (`diskd`'s device) go on it too.
pub fn dispatch() -> *mut Dispatch {
    init();
    DISPATCH.load(Ordering::Relaxed) as *mut Dispatch
}

/// What `umount()` does: `f(mount, force)`.
pub fn set_unmount(f: fn(usize, bool) -> Result<()>) {
    *UNMOUNT.lock().unwrap() = Some(f);
}

/// Serves `fs` at `path`, its root with permissions `mode`; `source` is
/// what it is mounted from (`/dev/md0`, `tmpfs`). The mount's number.
pub fn attach(path: &str, source: &str, fs: Box<dyn FileSystem>) -> Result<usize> {
    let dpp = dispatch();
    let c = CString::new(path).map_err(|_| e::EINVAL)?;
    let root = calloc_bytes(size_of::<IofuncAttr>()).cast::<IofuncAttr>();
    if root.is_null() {
        return Err(e::ENOMEM);
    }
    // SAFETY: allocated above, zeroed; it lives as long as the process.
    unsafe { iofunc_attr_init(root, S_IFDIR | 0o755, null(), null()) };
    let kind = fs.kind();
    let mount = Arc::new(Mount {
        fs: Mutex::new(fs),
        root: root as usize,
        path: path.into(),
        source: source.into(),
        id: AtomicI32::new(-1),
        open: AtomicI32::new(0),
    });
    let index = {
        let mut all = MOUNTS.lock().unwrap();
        all.push(Some(mount.clone()));
        all.len() - 1
    };
    let attr = ResmgrAttr { flags: 0, nparts_max: 1, msg_max_size: 8192, other_func: None, reserved: [0; 4] };
    const DIR: c_uint = 0x100;
    // SAFETY: the dispatch made above, with statics of this program and
    // the attribute just made.
    let id = unsafe { resmgr_attach(dpp, &attr, c.as_ptr(), 0, DIR, &raw const CONNECT, &raw const IO, root.cast()) };
    if id < 0 {
        MOUNTS.lock().unwrap()[index] = None;
        return Err(e::EIO);
    }
    mount.id.store(id, Ordering::Relaxed);
    record(true, format!("{source} {path} {kind}"));
    Ok(index)
}

/// Gives mount `index` up: its file system back. `EBUSY` while files are
/// open on it, unless `force`; the caller's own descriptor is one.
pub fn detach(index: usize, force: bool) -> Result<Box<dyn FileSystem>> {
    let mount = mount(index).ok_or(e::EINVAL)?;
    if !force && mount.open.load(Ordering::Relaxed) > 1 {
        return Err(e::EBUSY);
    }
    // SAFETY: the attachment `attach` made.
    unsafe { resmgr_detach(dispatch(), mount.id.swap(-1, Ordering::Relaxed), 0) };
    MOUNTS.lock().unwrap()[index] = None;
    let kind = mount.fs.lock().unwrap().kind();
    record(false, format!("{} {} {kind}", mount.source, mount.path));
    let mount = Arc::try_unwrap(mount).map_err(|_| e::EBUSY)?;
    Ok(mount.fs.into_inner().unwrap())
}

/// Answers messages, for ever.
pub fn serve() -> ! {
    let dpp = dispatch();
    // SAFETY: the framework's functions on our dispatch.
    unsafe {
        let ctp = dispatch_context_alloc(dpp);
        loop {
            if dispatch_block(ctp).is_null() {
                std::process::exit(1);
            }
            dispatch_handler(ctp);
        }
    }
}

/// The mount table's line, added or taken out. In a thread of its own: the
/// table may be on a file system this process serves.
fn record(add: bool, line: String) {
    std::thread::spawn(move || {
        let old = std::fs::read_to_string(MOUNT_TABLE).unwrap_or_default();
        let mut lines: Vec<&str> = old.lines().filter(|l| *l != line).collect();
        if add {
            lines.push(&line);
        }
        let _ = std::fs::create_dir_all("/var/run");
        let _ = std::fs::write(MOUNT_TABLE, lines.join("\n") + "\n");
    });
}

fn mount(index: usize) -> Option<Arc<Mount>> {
    MOUNTS.lock().unwrap().get(index).cloned().flatten()
}

/// The mount a connect message's handle names.
fn mount_of(handle: *mut c_void) -> Option<(usize, Arc<Mount>)> {
    let all = MOUNTS.lock().unwrap();
    all.iter().enumerate().find_map(|(i, m)| m.as_ref().filter(|m| m.root == handle as usize).map(|m| (i, m.clone())))
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The client's effective IDs and groups.
fn who(ctp: *mut Context) -> Who {
    let mut info = ClientInfo::default();
    // SAFETY: the context is the framework's.
    let scoid = unsafe { (*ctp).info.scoid };
    if ConnectClientInfo(scoid, (&raw mut info).cast(), 8) != 0 {
        return Who { uid: u32::MAX, gid: u32::MAX, groups: Vec::new() };
    }
    let n = (info.ngroups as usize).min(info.grouplist.len());
    Who { uid: info.euid, gid: info.egid, groups: info.grouplist[..n].to_vec() }
}

/// The path in a connect message, from the mount's root.
///
/// # Safety
///
/// `msg` is a connect message.
unsafe fn connect_path(msg: *const u8) -> String {
    // SAFETY: the caller's guarantee.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above; the framework received the path.
    let b = unsafe { core::slice::from_raw_parts(msg.add(size_of::<Connect>()), usize::from(m.path_len)) };
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    format!("/{}", String::from_utf8_lossy(&b[..end]).trim_start_matches('/').trim_end_matches('/'))
}

fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// May `who` add a name to, or take `gone` from, `path`'s directory: write
/// and search on it, and with the sticky bit the file's or the directory's
/// owner (`ufs_delete_denied()`).
fn may_change(fs: &mut dyn FileSystem, path: &str, who: &Who, gone: Option<&Node>) -> Result<()> {
    if path == "/" {
        return Err(e::EBUSY);
    }
    let dir = fs.lookup(parent(path))?;
    if !dir.is_dir() {
        return Err(e::ENOTDIR);
    }
    if !who.may(&dir, 3) {
        return Err(e::EACCES);
    }
    if let Some(n) = gone
        && dir.mode & S_ISVTX != 0
        && !who.is_root()
        && who.uid != n.uid
        && who.uid != dir.uid
    {
        return Err(e::EPERM);
    }
    Ok(())
}

/// What `stat()` will say, in the open's attribute.
fn fill(attr: *mut IofuncAttr, n: &Node) {
    // SAFETY: an attribute of this server.
    let a = unsafe { &mut *attr };
    a.mode = n.mode;
    a.uid = n.uid;
    a.gid = n.gid;
    a.nlink = n.nlink.max(1);
    a.inode = n.ino;
    a.nbytes = n.size as i64;
    a.atime = n.atime;
    a.mtime = n.mtime;
    a.ctime = n.ctime;
    // The file system's times are the truth: not "now", as the framework's
    // pending-update flags would make them.
    a.flags &= !ATTR_TIMES;
}

/// Binds an open of `path` to the client.
fn bind(ctp: *mut Context, msg: *mut u8, index: usize, mount: &Mount, path: String, n: &Node) -> c_int {
    let attr = calloc_bytes(size_of::<IofuncAttr>()).cast::<IofuncAttr>();
    let ocb = calloc_bytes(size_of::<Ocb>()).cast::<Ocb>();
    if attr.is_null() || ocb.is_null() {
        return e::ENOMEM;
    }
    // SAFETY: allocated above, zeroed.
    unsafe {
        iofunc_attr_init(attr, n.mode, null(), null());
        (*ocb).attr = attr;
        (*ocb).path = Box::into_raw(Box::new(path));
        (*ocb).mount = index;
    }
    fill(attr, n);
    mount.open.fetch_add(1, Ordering::Relaxed);
    // SAFETY: the framework's function, with its message and our OCB.
    unsafe { iofunc_ocb_attach(ctp, msg, ocb.cast(), attr, null()) }
}

/// `open()`: `O_CREAT`, `O_TRUNC` and the access here, the rest by the
/// framework.
unsafe extern "C" fn io_open(ctp: *mut Context, msg: *mut u8, h: *mut c_void, _x: *mut c_void) -> c_int {
    let Some((index, mount)) = mount_of(h) else { return e::ENOENT };
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above.
    let path = unsafe { connect_path(msg) };
    let who = who(ctp);
    let (rd, wr) = (m.ioflag & IO_FLAG_RD != 0, m.ioflag & IO_FLAG_WR != 0);
    let t = now();
    let found = {
        let mut fs = mount.fs.lock().unwrap();
        let fs = &mut **fs;
        let (n, made) = match fs.lookup(&path) {
            Ok(_) if m.ioflag & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL => return e::EEXIST,
            Ok(n) => (n, false),
            Err(err) if err == e::ENOENT && m.ioflag & O_CREAT != 0 => {
                if let Err(err) = may_change(fs, &path, &who, None) {
                    return err;
                }
                match fs.make(&path, S_IFREG | (m.mode & 0o7777), &who, t) {
                    Ok(n) => (n, true),
                    Err(err) => return err,
                }
            }
            Err(err) => return err,
        };
        if n.is_dir() && wr {
            return e::EISDIR;
        }
        if m.ioflag & O_DIRECTORY != 0 && !n.is_dir() {
            return e::ENOTDIR;
        }
        // A socket's node opens for writing only: a client's `connect()`
        // hands it to the stack, which checks that access (io-sock's
        // `verify_handle`, read with REA).
        if n.mode & S_IFMT == S_IFSOCK && rd {
            return e::EOPNOTSUPP;
        }
        if !made && !who.may(&n, if rd { 4 } else { 0 } | if wr { 2 } else { 0 }) {
            return e::EACCES;
        }
        let mut n = n;
        if m.ioflag & O_TRUNC != 0 && wr && n.mode & S_IFMT == S_IFREG && n.size != 0 {
            match fs.truncate(&path, 0, t) {
                Ok(s) => n.size = s,
                Err(err) => return err,
            }
        }
        n
    };
    bind(ctp, msg, index, &mount, path, &found)
}

/// `mkdir()`, and a socket's node, which is opened for the client as an
/// open would be (`libsocket`'s and libsys's `bind()`).
unsafe extern "C" fn io_mknod(ctp: *mut Context, msg: *mut u8, h: *mut c_void, _x: *mut c_void) -> c_int {
    let Some((index, mount)) = mount_of(h) else { return e::ENOENT };
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above.
    let path = unsafe { connect_path(msg) };
    let who = who(ctp);
    let made = {
        let mut fs = mount.fs.lock().unwrap();
        let fs = &mut **fs;
        if fs.lookup(&path).is_ok() {
            return e::EEXIST;
        }
        if let Err(err) = may_change(fs, &path, &who, None) {
            return err;
        }
        fs.make(&path, m.mode, &who, now())
    };
    match made {
        Ok(n) if n.mode & S_IFMT == S_IFSOCK && m.ioflag & (IO_FLAG_RD | IO_FLAG_WR) != 0 => {
            bind(ctp, msg, index, &mount, path, &n)
        }
        Ok(_) => 0,
        Err(err) => err,
    }
}

/// `unlink()` and `rmdir()`: the message's mode says which.
unsafe extern "C" fn io_unlink(ctp: *mut Context, msg: *mut u8, h: *mut c_void, _x: *mut c_void) -> c_int {
    let Some((_, mount)) = mount_of(h) else { return e::ENOENT };
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above.
    let path = unsafe { connect_path(msg) };
    let who = who(ctp);
    let mut fs = mount.fs.lock().unwrap();
    let fs = &mut **fs;
    let done = fs.lookup(&path).and_then(|n| {
        may_change(fs, &path, &who, Some(&n))?;
        fs.remove(&path, m.mode & S_IFMT == S_IFDIR, now())
    });
    done.map_or_else(|err| err, |()| 0)
}

/// `rename()`: the new path, then the old in the extra data.
unsafe extern "C" fn io_rename(ctp: *mut Context, msg: *mut u8, h: *mut c_void, _x: *mut c_void) -> c_int {
    let Some((_, mount)) = mount_of(h) else { return e::ENOENT };
    // SAFETY: a connect message.
    let m = unsafe { msg.cast::<Connect>().read_unaligned() };
    // SAFETY: as above.
    let to = unsafe { connect_path(msg) };
    let at = (size_of::<Connect>() + usize::from(m.path_len) + 7) & !7;
    // SAFETY: the framework received the extra data with the message.
    let b = unsafe { core::slice::from_raw_parts(msg.add(at), usize::from(m.extra_len)) };
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    let from = format!("/{}", String::from_utf8_lossy(&b[..end]).trim_start_matches('/').trim_end_matches('/'));
    let who = who(ctp);
    let mut fs = mount.fs.lock().unwrap();
    let fs = &mut **fs;
    let done = fs.lookup(&from).and_then(|n| {
        may_change(fs, &from, &who, Some(&n))?;
        let old = fs.lookup(&to).ok();
        may_change(fs, &to, &who, old.as_ref())?;
        fs.rename(&from, &to, now())
    });
    done.map_or_else(|err| err, |()| 0)
}

/// # Safety
///
/// `ocb` is one of this server's.
unsafe fn open_file(ocb: *mut c_void) -> (String, Option<Arc<Mount>>) {
    // SAFETY: the caller's guarantee.
    unsafe { ((*(*ocb.cast::<Ocb>()).path).clone(), mount((*ocb.cast::<Ocb>()).mount)) }
}

/// `f` on the file system of an open file: `ENXIO` once it is unmounted.
///
/// # Safety
///
/// `ocb` is one of this server's.
unsafe fn on<R>(ocb: *mut c_void, f: impl FnOnce(&mut dyn FileSystem, &str) -> Result<R>) -> Result<R> {
    // SAFETY: the caller's guarantee.
    let (path, mount) = unsafe { open_file(ocb) };
    let mount = mount.ok_or(e::ENXIO)?;
    let mut fs = mount.fs.lock().unwrap();
    f(&mut **fs, &path)
}

fn reply(ctp: *mut Context, status: usize, data: &[u8]) -> c_int {
    // SAFETY: the context is the framework's.
    let rcvid = unsafe { (*ctp).rcvid };
    MsgReply(rcvid, status as c_long, data.as_ptr().cast(), data.len());
    RESMGR_NOREPLY
}

/// FreeBSD's `struct dirent` of a name.
fn dirent(ino: u64, off: u64, name: &str, mode: u32, out: &mut Vec<u8>) {
    let kind = match mode & S_IFMT {
        S_IFDIR => 4,
        S_IFREG => 8,
        S_IFSOCK => 12,
        0o120000 => 10,
        0o010000 => 1,
        _ => 0,
    };
    let reclen = (24 + name.len() + 1).next_multiple_of(8);
    let at = out.len();
    out.resize(at + reclen, 0);
    let r = &mut out[at..];
    r[0..8].copy_from_slice(&ino.to_le_bytes());
    r[8..16].copy_from_slice(&off.to_le_bytes());
    r[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
    r[18] = kind;
    r[20..22].copy_from_slice(&(name.len() as u16).to_le_bytes());
    r[24..24 + name.len()].copy_from_slice(name.as_bytes());
}

fn read_dir(ctp: *mut Context, ocb: *mut c_void, room: usize) -> c_int {
    // SAFETY: an OCB of this server.
    let listed = unsafe { on(ocb, |fs, p| Ok((fs.lookup(p)?, fs.lookup(parent(p))?, fs.list(p)?))) };
    let (me, up, list) = match listed {
        Ok(l) => l,
        Err(err) => return err,
    };
    // SAFETY: as above.
    let o = unsafe { &mut (*ocb.cast::<Ocb>()).ocb };
    let mut out = Vec::new();
    let total = list.len() as i64 + 2;
    while o.offset < total {
        let at = o.offset;
        let before = out.len();
        match at {
            0 => dirent(me.ino, 1, ".", S_IFDIR, &mut out),
            1 => dirent(up.ino, 2, "..", S_IFDIR, &mut out),
            n => {
                let (name, f) = &list[n as usize - 2];
                dirent(f.ino, n as u64 + 1, name, f.mode, &mut out);
            }
        }
        if out.len() > room {
            out.truncate(before);
            break;
        }
        o.offset += 1;
    }
    reply(ctp, out.len(), &out)
}

/// A message's `_IO_XTYPE_OFFSET`: `pread()` and `pwrite()`'s place,
/// which leaves the file's own alone. `Err` for any other extended type.
///
/// # Safety
///
/// `msg` is the message `m` came from.
unsafe fn given_offset(m: &io::ReadWrite, msg: *const u8) -> Result<Option<u64>> {
    match m.xtype & XTYPE_MASK {
        0 => Ok(None),
        io::IO_XTYPE_OFFSET => {
            // SAFETY: the offset follows the header.
            Ok(Some(unsafe { msg.add(size_of::<io::ReadWrite>()).cast::<u64>().read_unaligned() }))
        }
        _ => Err(e::ENOSYS),
    }
}

/// `read()`, and a directory's `getdirentries()`.
unsafe extern "C" fn io_read(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a read message.
    let m = unsafe { msg.cast::<io::ReadWrite>().read_unaligned() };
    if m.xtype & XTYPE_MASK == XTYPE_READDIR {
        return read_dir(ctp, ocb, m.nbytes as usize);
    }
    // SAFETY: the message's buffer.
    let given = match unsafe { given_offset(&m, msg) } {
        Ok(g) => g,
        Err(err) => return err,
    };
    // SAFETY: an OCB of this server.
    let at = given.unwrap_or(unsafe { (*ocb.cast::<Ocb>()).ocb.offset as u64 });
    let mut buf = vec![0u8; (m.nbytes as usize).min(1 << 20)];
    // SAFETY: as above.
    match unsafe { on(ocb, |fs, p| fs.read(p, at, &mut buf)) } {
        Ok(n) => {
            if given.is_none() {
                // SAFETY: as above.
                unsafe { (*ocb.cast::<Ocb>()).ocb.offset += n as i64 };
            }
            reply(ctp, n, &buf[..n])
        }
        Err(err) => err,
    }
}

/// `write()`.
unsafe extern "C" fn io_write(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a write message.
    let m = unsafe { msg.cast::<io::ReadWrite>().read_unaligned() };
    let n = m.nbytes as usize;
    let mut data = vec![0u8; n];
    // SAFETY: the context is the framework's.
    let rcvid = unsafe { (*ctp).rcvid };
    // SAFETY: the message's buffer.
    let given = match unsafe { given_offset(&m, msg) } {
        Ok(g) => g,
        Err(err) => return err,
    };
    let data_at = size_of::<io::ReadWrite>() + if given.is_some() { 8 } else { 0 };
    if MsgRead(rcvid, data.as_mut_ptr().cast(), n, data_at) < 0 {
        return e::EFAULT;
    }
    // SAFETY: an OCB of this server.
    let o = unsafe { &mut (*ocb.cast::<Ocb>()).ocb };
    let at = match given {
        Some(at) => Some(at),
        None if o.ioflag as u32 & O_APPEND != 0 => None,
        None => Some(o.offset as u64),
    };
    // SAFETY: as above.
    match unsafe { on(ocb, |fs, p| fs.write(p, at, &data, now())) } {
        Ok((at, size)) => {
            if given.is_none() {
                o.offset = at as i64 + n as i64;
            }
            // SAFETY: as above.
            unsafe { (*(*ocb.cast::<Ocb>()).attr).nbytes = size as i64 };
            reply(ctp, n, &[])
        }
        Err(err) => err,
    }
}

/// `stat()`: as the file is now, by the framework's default.
unsafe extern "C" fn io_stat(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this server.
    if let Ok(n) = unsafe { on(ocb, |fs, p| fs.lookup(p)) } {
        // SAFETY: as above.
        fill(unsafe { (*ocb.cast::<Ocb>()).attr }, &n);
    }
    // SAFETY: the framework's function with its arguments.
    unsafe { iofunc_stat_default(ctp, msg, ocb.cast()) }
}

/// `ftruncate()`: `F_FREESP64` from a place to the end, `F_GROWSP64`.
unsafe extern "C" fn io_space(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a space message.
    let m = unsafe { msg.cast::<io::Space>().read_unaligned() };
    // SAFETY: an OCB of this server.
    let size = unsafe {
        on(ocb, |fs, p| {
            let n = fs.lookup(p)?;
            let want = match m.subtype {
                F_FREESP64 => m.start.min(n.size),
                F_GROWSP64 => m.start + m.len,
                _ => return Err(e::EINVAL),
            };
            fs.truncate(p, want, now())
        })
    };
    match size {
        Ok(s) => reply(ctp, 0, &s.to_le_bytes()),
        Err(err) => err,
    }
}

/// `fsync()`.
unsafe extern "C" fn io_sync(_ctp: *mut Context, _msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this server.
    unsafe { on(ocb, |fs, _| fs.sync()) }.map_or_else(|err| err, |()| 0)
}

/// The last close: what it changed reaches the file system's store.
unsafe extern "C" fn io_close_ocb(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an OCB of this server.
    if let (_, Some(mount)) = unsafe { open_file(ocb) } {
        mount.open.fetch_sub(1, Ordering::Relaxed);
        let _ = mount.fs.lock().unwrap().sync();
    }
    // SAFETY: an OCB of this server, closing: its parts go.
    unsafe { drop(Box::from_raw((*ocb.cast::<Ocb>()).path)) };
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_close_ocb_default(ctp, msg, ocb.cast()) }
}

unsafe extern "C" fn io_chmod(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a chmod message.
    let m = unsafe { msg.cast::<io::Chmod>().read_unaligned() };
    let who = who(ctp);
    // SAFETY: an OCB of this server.
    unsafe { on(ocb, |fs, p| fs.set_mode(p, m.mode, &who, now())) }.map_or_else(|err| err, |()| 0)
}

unsafe extern "C" fn io_chown(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a chown message.
    let m = unsafe { msg.cast::<io::Chown>().read_unaligned() };
    let who = who(ctp);
    // SAFETY: an OCB of this server.
    unsafe { on(ocb, |fs, p| fs.set_owner(p, m.uid as u32, m.gid as u32, &who, now())) }.map_or_else(|err| err, |()| 0)
}

/// `futimens()` and `utimes()`.
unsafe extern "C" fn io_utime(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    const UTIME_NOW: i64 = -1;
    const UTIME_OMIT: i64 = -2;
    // SAFETY: the first words of either form.
    let kind = unsafe { msg.cast::<u16>().read_unaligned() };
    let t = now();
    let (atime, mtime, to_now) = if kind == io::IO_UTIME64 {
        // SAFETY: a utime message.
        let m = unsafe { msg.cast::<io::Utime64>().read_unaligned() };
        let pick = |s: i64, ns: i64| match ns {
            UTIME_OMIT => None,
            UTIME_NOW => Some(t),
            _ => Some(s),
        };
        let to_now = m.atime_ns == UTIME_NOW && m.mtime_ns == UTIME_NOW;
        (pick(m.atime_s, m.atime_ns), pick(m.mtime_s, m.mtime_ns), to_now)
    } else {
        // SAFETY: as above.
        let m = unsafe { msg.cast::<io::Utime>().read_unaligned() };
        if m.cur_flag != 0 { (Some(t), Some(t), true) } else { (Some(i64::from(m.actime)), Some(i64::from(m.modtime)), false) }
    };
    let who = who(ctp);
    // SAFETY: an OCB of this server.
    unsafe { on(ocb, |fs, p| fs.set_times(p, atime, mtime, &who, to_now)) }.map_or_else(|err| err, |()| 0)
}

/// `iofdinfo()`: the open's `struct _fdinfo`, and its path, the mount's
/// and the file's (libsys resolves a relative `*at()` path from it).
unsafe extern "C" fn io_fdinfo(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: an fdinfo message.
    let req = unsafe { msg.cast::<io::FdinfoReq>().read_unaligned() };
    let mut out = vec![0u8; 0x48];
    // SAFETY: the reply's `struct _fdinfo`, after its 8-byte header.
    unsafe { iofunc_fdinfo(ctp, ocb.cast(), core::ptr::null_mut(), out.as_mut_ptr().add(8).cast()) };
    // SAFETY: an OCB of this server.
    let (path, mount) = unsafe { open_file(ocb) };
    let Some(mount) = mount else { return e::ENXIO };
    let mut full = if path == "/" { mount.path.clone() } else { format!("{}{path}", mount.path.trim_end_matches('/')) };
    full.push('\0');
    let mut n = 0;
    if req.flags & 2 == 0 {
        n = full.len().min(req.path_len as usize);
        out.extend_from_slice(&full.as_bytes()[..n]);
    }
    reply(ctp, n, &out)
}

fn devctl_reply(ctp: *mut Context, data: &[u8]) -> c_int {
    let mut out = vec![0u8; size_of::<io::DevctlReply>() + data.len()];
    out[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
    out[size_of::<io::DevctlReply>()..].copy_from_slice(data);
    reply(ctp, 0, &out)
}

/// A mount's `statfs()`, its names, `unmount()`, and the common commands.
unsafe extern "C" fn io_devctl(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a devctl message.
    let m = unsafe { msg.cast::<io::Devctl>().read_unaligned() };
    // SAFETY: an OCB of this server.
    let index = unsafe { (*ocb.cast::<Ocb>()).mount };
    let Some(mount) = mount(index) else { return e::ENXIO };
    match (m.dcmd as u32) & 0xffff {
        DCMD_UNMOUNT => {
            // SAFETY: an int follows the header.
            let force = unsafe { msg.add(size_of::<io::Devctl>()).cast::<c_int>().read_unaligned() } != 0;
            drop(mount);
            let hook = *UNMOUNT.lock().unwrap();
            match hook {
                Some(f) => f(index, force).map_or_else(|err| err, |()| devctl_reply(ctp, &[])),
                None => e::EBUSY,
            }
        }
        DCMD_NAMES => devctl_reply(ctp, format!("{}\0{}\0", mount.source, mount.path).as_bytes()),
        DCMD_STATVFS => {
            let mut fs = mount.fs.lock().unwrap();
            let kind = fs.kind();
            match fs.space() {
                Ok(s) => {
                    let mut v = [0u8; 168];
                    v[0..4].copy_from_slice(&s.block_size.to_le_bytes());
                    v[4..8].copy_from_slice(&s.block_size.to_le_bytes());
                    v[8..16].copy_from_slice(&s.blocks.to_le_bytes());
                    v[16..24].copy_from_slice(&s.free.to_le_bytes());
                    v[24..32].copy_from_slice(&s.free.to_le_bytes());
                    v[32..40].copy_from_slice(&s.files.to_le_bytes());
                    v[40..48].copy_from_slice(&s.free_files.to_le_bytes());
                    v[48..56].copy_from_slice(&s.free_files.to_le_bytes());
                    let k = kind.as_bytes();
                    v[60..60 + k.len().min(15)].copy_from_slice(&k[..k.len().min(15)]);
                    v[80..84].copy_from_slice(&255u32.to_le_bytes());
                    devctl_reply(ctp, &v)
                }
                Err(err) => err,
            }
        }
        // The rest (`fcntl()`'s flags) as any server's.
        // SAFETY: the framework's default with its own arguments.
        _ => unsafe { iofunc_devctl_default(ctp, msg, ocb.cast()) },
    }
}

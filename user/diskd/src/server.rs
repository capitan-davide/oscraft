//! The disk's resource manager: its blocks as `/dev/md<unit>`, and the FAT
//! file system on it, served by `fsd`, while mounted.

use core::ffi::{c_char, c_int, c_long, c_void};
use core::ptr::null;
use std::ffi::CString;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use bufcache::{BlockDevice, Cache};
use fsd::{FileSystem, Node, S_IFDIR, S_IFMT, S_IFREG, Space, Who};
use kercall::neutrino::msg::{MsgRead, MsgReply};
use msdosfs::{ATTR_READONLY, DosTime, Entry, Error, Volume};
use proto::errno as e;
use proto::io;
use resmgr::abi::{ConnectFuncs, Context, IoFuncs, IofuncAttr, IofuncOcb, RESMGR_NOREPLY, ResmgrAttr};
use resmgr::iofunc::{iofunc_attr_init, iofunc_close_ocb_default, iofunc_func_init, iofunc_ocb_attach};
use resmgr::mem::calloc_bytes;
use resmgr::resmgr::resmgr_attach;

struct Port(Box<dyn BlockDevice + Send>);

impl BlockDevice for Port {
    fn block_size(&self) -> usize {
        self.0.block_size()
    }
    fn blocks(&self) -> u64 {
        self.0.blocks()
    }
    fn read(&mut self, block: u64, buf: &mut [u8]) -> std::io::Result<()> {
        self.0.read(block, buf)
    }
    fn write(&mut self, block: u64, buf: &[u8]) -> std::io::Result<()> {
        self.0.write(block, buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// The disk while raw; `None` while mounted, its blocks then the file
/// system's.
static DISK: Mutex<Option<Cache<Port>>> = Mutex::new(None);
static NAME: Mutex<String> = Mutex::new(String::new());
/// The mount's number in `fsd`, while mounted.
static MOUNTED: Mutex<Option<usize>> = Mutex::new(None);
static RAW_OPEN: AtomicUsize = AtomicUsize::new(0);
/// `diskd`'s command, and those of `<sys/disk.h>`.
const DCMD_MOUNT: u32 = 0x4d01;
const DIOCGSECTORSIZE: u32 = 0x6480;
const DIOCGMEDIASIZE: u32 = 0x6481;
const DIOCGFWSECTORS: u32 = 0x6482;
const DIOCGFWHEADS: u32 = 0x6483;
const CACHE_BLOCKS: usize = 2048;
const F_READ: usize = 0;
const F_WRITE: usize = 1;
const F_CLOSE_OCB: usize = 2;
const F_DEVCTL: usize = 5;
const C_OPEN: usize = 0;

/// An open of the device.
#[repr(C)]
struct Ocb {
    ocb: IofuncOcb,
    attr: *mut IofuncAttr,
}

/// `msdosfs`'s errors as FreeBSD's numbers.
fn errno(err: Error) -> c_int {
    match err {
        Error::NotFound => e::ENOENT,
        Error::Exists => e::EEXIST,
        Error::NotDir => e::ENOTDIR,
        Error::IsDir => e::EISDIR,
        Error::NotEmpty => e::ENOTEMPTY,
        Error::NoSpace => e::ENOSPC,
        Error::NameTooLong => e::ENAMETOOLONG,
        Error::Invalid => e::EINVAL,
        Error::Busy => e::EBUSY,
        Error::Io => e::EIO,
    }
}

/// Seconds since the Epoch in FAT's words: FAT keeps local time, and this
/// system's is UTC. It starts in 1980.
fn dos_time(secs: u64) -> DosTime {
    let days = (secs / 86400) as i64;
    let (z, rest) = (days + 719_468, secs % 86400);
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    DosTime {
        date: (((year - 1980).clamp(0, 127) as u16) << 9) | ((month as u16) << 5) | day as u16,
        time: (((rest / 3600) as u16) << 11) | (((rest / 60 % 60) as u16) << 5) | ((rest % 60 / 2) as u16),
    }
}


/// Seconds since the Epoch of a FAT time.
fn secs_of(t: DosTime) -> i64 {
    let (y, m, d) = (i64::from(t.date >> 9) + 1980, i64::from(t.date >> 5 & 15), i64::from(t.date & 31));
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86400 + i64::from(t.time >> 11) * 3600 + i64::from(t.time >> 5 & 63) * 60 + i64::from(t.time & 31) * 2
}

/// FAT behind `fsd`: no owners (root's), no other file types, and the
/// owner's write bit in the read-only attribute (`msdosfs_getattr()`,
/// `msdosfs_setattr()`).
struct Fat(Volume<Port>);

impl Fat {
    fn node(path: &str, f: &Entry) -> Node {
        let mut mode = if f.is_dir() { S_IFDIR | 0o755 } else { S_IFREG | 0o755 };
        if f.attr & ATTR_READONLY != 0 {
            mode &= !0o222;
        }
        // A file without clusters has no number of its own: its path's.
        let ino = if f.first_cluster != 0 {
            u64::from(f.first_cluster)
        } else if path == "/" {
            1
        } else {
            path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)) | 1 << 40
        };
        let t = secs_of(f.mtime);
        Node { ino, mode, uid: 0, gid: 0, nlink: 1, size: u64::from(f.size), atime: t, mtime: t, ctime: t }
    }
}

fn dos_of(secs: i64) -> DosTime {
    dos_time(secs.max(0) as u64)
}

impl FileSystem for Fat {
    fn kind(&self) -> &'static str {
        "msdosfs"
    }
    fn lookup(&mut self, path: &str) -> fsd::Result<Node> {
        self.0.resolve(path).map(|f| Fat::node(path, &f)).map_err(errno)
    }
    fn make(&mut self, path: &str, mode: u32, _who: &Who, now: i64) -> fsd::Result<Node> {
        let f = match mode & S_IFMT {
            S_IFREG => self.0.create(path, dos_of(now)),
            S_IFDIR => self.0.mkdir(path, dos_of(now)),
            _ => return Err(e::ENOTSUP),
        };
        let f = f.and_then(|f| self.0.sync().map(|()| f)).map_err(errno)?;
        Ok(Fat::node(path, &f))
    }
    fn read(&mut self, path: &str, at: u64, buf: &mut [u8]) -> fsd::Result<usize> {
        let f = self.0.resolve(path).map_err(errno)?;
        self.0.read_at(&f, at, buf).map_err(errno)
    }
    fn write(&mut self, path: &str, at: Option<u64>, data: &[u8], now: i64) -> fsd::Result<(u64, u64)> {
        let mut f = self.0.resolve(path).map_err(errno)?;
        let at = at.unwrap_or(u64::from(f.size));
        self.0.write_at(&mut f, at, data, dos_of(now)).map_err(errno)?;
        Ok((at, u64::from(f.size)))
    }
    fn truncate(&mut self, path: &str, size: u64, now: i64) -> fsd::Result<u64> {
        let mut f = self.0.resolve(path).map_err(errno)?;
        let size = u32::try_from(size).map_err(|_| e::EFBIG)?;
        self.0.truncate(&mut f, size, dos_of(now)).map_err(errno)?;
        Ok(u64::from(f.size))
    }
    fn list(&mut self, path: &str) -> fsd::Result<Vec<(String, Node)>> {
        let d = self.0.resolve(path).map_err(errno)?;
        let all = self.0.read_dir(&d).map_err(errno)?;
        let base = path.trim_end_matches('/');
        Ok(all
            .into_iter()
            .filter(|f| f.name != "." && f.name != "..")
            .map(|f| {
                let n = Fat::node(&format!("{base}/{}", f.name), &f);
                (f.name, n)
            })
            .collect())
    }
    fn remove(&mut self, path: &str, dir: bool, _now: i64) -> fsd::Result<()> {
        let r = if dir { self.0.rmdir(path) } else { self.0.unlink(path) };
        r.and_then(|()| self.0.sync()).map_err(errno)
    }
    fn rename(&mut self, from: &str, to: &str, now: i64) -> fsd::Result<()> {
        self.0.rename(from, to, dos_of(now)).and_then(|()| self.0.sync()).map_err(errno)
    }
    /// The mount's owner, root here, or root's privilege, which are one.
    fn set_mode(&mut self, path: &str, mode: u32, who: &Who, _now: i64) -> fsd::Result<()> {
        if !who.is_root() {
            return Err(e::EPERM);
        }
        self.0.set_mode(path, mode).map_err(errno)
    }
    /// FAT has no owners. Asking for the mount's is fine; anyone else is
    /// told EPERM, root EINVAL (`msdosfs_setattr()`).
    fn set_owner(&mut self, path: &str, uid: u32, gid: u32, who: &Who, _now: i64) -> fsd::Result<()> {
        self.0.resolve(path).map_err(errno)?;
        let keeps = |id: u32| id == 0 || id == u32::MAX;
        match (keeps(uid) && keeps(gid), who.is_root()) {
            (true, true) => Ok(()),
            (_, false) => Err(e::EPERM),
            (false, true) => Err(e::EINVAL),
        }
    }
    /// The modification time; FAT's access time isn't kept here.
    fn set_times(&mut self, path: &str, _atime: Option<i64>, mtime: Option<i64>, who: &Who, _to_now: bool) -> fsd::Result<()> {
        if !who.is_root() {
            return Err(e::EPERM);
        }
        match mtime {
            Some(t) => self.0.set_mtime(path, dos_of(t)).map_err(errno),
            None => Ok(()),
        }
    }
    fn space(&mut self) -> fsd::Result<Space> {
        let (cluster, total, free) = self.0.space().map_err(errno)?;
        Ok(Space { block_size: cluster, blocks: u64::from(total), free: u64::from(free), files: 0, free_files: 0 })
    }
    fn sync(&mut self) -> fsd::Result<()> {
        self.0.sync().map_err(errno)
    }
}

// ANCHOR: diskd_mount
/// Takes the disk as a file system, and serves `path`.
fn mount(path: &str) -> Result<(), c_int> {
    let mut g = DISK.lock().unwrap();
    let Some(mut cache) = g.take() else {
        return Err(e::EBUSY);
    };
    // A disk that isn't FAT stays what it was. (`mount` asks through an
    // open of the device: it may be open.)
    if !Volume::recognises(&mut cache) {
        *g = Some(cache);
        return Err(e::EINVAL);
    }
    let volume = Volume::mount(cache).map_err(errno)?;
    let name = NAME.lock().unwrap().clone();
    let index = fsd::attach(path, &name, Box::new(Fat(volume)))?;
    *MOUNTED.lock().unwrap() = Some(index);
    Ok(())
}
// ANCHOR_END: diskd_mount

/// `umount()`: the path given up, the disk raw again.
fn unmount(index: usize, force: bool) -> Result<(), c_int> {
    let fs = fsd::detach(index, force)?;
    let any: Box<dyn core::any::Any> = fs;
    let Ok(fat) = any.downcast::<Fat>() else {
        return Err(e::EINVAL);
    };
    let port = fat.0.unmount().map_err(errno)?;
    *DISK.lock().unwrap() = Some(Cache::new(port, CACHE_BLOCKS));
    *MOUNTED.lock().unwrap() = None;
    Ok(())
}

fn reply(ctp: *mut Context, status: usize, data: &[u8]) -> c_int {
    // SAFETY: the context is the framework's.
    let rcvid = unsafe { (*ctp).rcvid };
    MsgReply(rcvid, status as c_long, data.as_ptr().cast(), data.len());
    RESMGR_NOREPLY
}

/// A devctl's reply, with `data`.
fn devctl_reply(ctp: *mut Context, data: &[u8]) -> c_int {
    let mut out = vec![0u8; size_of::<io::DevctlReply>() + data.len()];
    out[8..12].copy_from_slice(&(data.len() as u32).to_le_bytes());
    out[size_of::<io::DevctlReply>()..].copy_from_slice(data);
    reply(ctp, 0, &out)
}

/// `/dev/md0`: its blocks.
unsafe extern "C" fn raw_open(ctp: *mut Context, msg: *mut u8, _h: *mut c_void, _x: *mut c_void) -> c_int {
    if DISK.lock().unwrap().is_none() {
        return e::EBUSY;
    }
    let attr = calloc_bytes(size_of::<IofuncAttr>()).cast::<IofuncAttr>();
    let ocb = calloc_bytes(size_of::<Ocb>()).cast::<Ocb>();
    if attr.is_null() || ocb.is_null() {
        return e::ENOMEM;
    }
    // SAFETY: allocated above, zeroed.
    unsafe {
        iofunc_attr_init(attr, 0o020666, null(), null());
        (*ocb).attr = attr;
    }
    RAW_OPEN.fetch_add(1, Ordering::Relaxed);
    // SAFETY: the framework's function, with its message and our OCB.
    unsafe { iofunc_ocb_attach(ctp, msg, ocb.cast(), attr, null()) }
}

fn raw<R>(f: impl FnOnce(&mut Cache<Port>) -> std::io::Result<R>) -> Result<R, c_int> {
    match DISK.lock().unwrap().as_mut() {
        Some(c) => f(c).map_err(|_| e::EIO),
        None => Err(e::EBUSY),
    }
}

/// Bytes at `at`, through the cache.
unsafe extern "C" fn raw_read(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a read message.
    let m = unsafe { msg.cast::<io::ReadWrite>().read_unaligned() };
    // SAFETY: an OCB of this server.
    let o = unsafe { &mut (*ocb.cast::<Ocb>()).ocb };
    let mut out = vec![0u8; (m.nbytes as usize).min(1 << 20)];
    let at = o.offset as u64;
    let done = raw(|c| {
        let (bs, blocks) = (c.block_size() as u64, c.blocks());
        let mut done = 0;
        while done < out.len() && (at + done as u64) / bs < blocks {
            let pos = at + done as u64;
            let inner = (pos % bs) as usize;
            let n = (out.len() - done).min(bs as usize - inner);
            let b = c.read(pos / bs)?;
            out[done..done + n].copy_from_slice(&b[inner..inner + n]);
            done += n;
        }
        Ok(done)
    });
    match done {
        Ok(n) => {
            o.offset += n as i64;
            reply(ctp, n, &out[..n])
        }
        Err(err) => err,
    }
}

unsafe extern "C" fn raw_write(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    // SAFETY: a write message.
    let m = unsafe { msg.cast::<io::ReadWrite>().read_unaligned() };
    let n = m.nbytes as usize;
    let mut data = vec![0u8; n];
    // SAFETY: the context is the framework's.
    let rcvid = unsafe { (*ctp).rcvid };
    if MsgRead(rcvid, data.as_mut_ptr().cast(), n, size_of::<io::ReadWrite>()) < 0 {
        return e::EFAULT;
    }
    // SAFETY: an OCB of this server.
    let o = unsafe { &mut (*ocb.cast::<Ocb>()).ocb };
    let at = o.offset as u64;
    let done = raw(|c| {
        let bs = c.block_size() as u64;
        let mut done = 0;
        while done < n {
            let pos = at + done as u64;
            let inner = (pos % bs) as usize;
            let len = (n - done).min(bs as usize - inner);
            let piece = &data[done..done + len];
            c.change(pos / bs, |b| b[inner..inner + len].copy_from_slice(piece))?;
            done += len;
        }
        c.sync()
    });
    match done {
        Ok(()) => {
            o.offset += n as i64;
            reply(ctp, n, &[])
        }
        Err(err) => err,
    }
}

unsafe extern "C" fn raw_devctl(ctp: *mut Context, msg: *mut u8, _ocb: *mut c_void) -> c_int {
    // SAFETY: a devctl message.
    let m = unsafe { msg.cast::<io::Devctl>().read_unaligned() };
    let data = msg.wrapping_add(size_of::<io::Devctl>());
    let (bs, blocks) = match raw(|c| Ok((c.block_size() as u64, c.blocks()))) {
        Ok(v) => v,
        Err(err) => return err,
    };
    match (m.dcmd as u32) & 0xffff {
        DIOCGSECTORSIZE => devctl_reply(ctp, &(bs as u32).to_le_bytes()),
        DIOCGMEDIASIZE => devctl_reply(ctp, &(bs * blocks).to_le_bytes()),
        // As FreeBSD's md, a geometry of no meaning.
        DIOCGFWSECTORS => devctl_reply(ctp, &32u32.to_le_bytes()),
        DIOCGFWHEADS => devctl_reply(ctp, &64u32.to_le_bytes()),
        DCMD_MOUNT => {
            // SAFETY: the path follows the header, in the message.
            let c = unsafe { core::ffi::CStr::from_ptr(data.cast()) }.to_string_lossy().into_owned();
            mount(&c).map_or_else(|err| err, |()| devctl_reply(ctp, &[]))
        }
        _ => e::ENOTTY,
    }
}

/// `/dev/md0`'s last close.
unsafe extern "C" fn raw_close(ctp: *mut Context, msg: *mut u8, ocb: *mut c_void) -> c_int {
    let _ = raw(Cache::sync);
    RAW_OPEN.fetch_sub(1, Ordering::Relaxed);
    // SAFETY: the framework's default with its own arguments.
    unsafe { iofunc_close_ocb_default(ctp, msg, ocb.cast()) }
}

// SAFETY: all zeroes is a valid start for these; `run` fills them.
static mut RAW_CONNECT: ConnectFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut RAW_IO: IoFuncs = unsafe { core::mem::zeroed() };
// SAFETY: as above.
static mut DEVICE: IofuncAttr = unsafe { core::mem::zeroed() };

/// Serves `device` as `name`, and mounted at `path` if given, for ever.
pub fn run(device: Box<dyn BlockDevice + Send>, name: &str, path: Option<&str>) {
    *DISK.lock().unwrap() = Some(Cache::new(Port(device), CACHE_BLOCKS));
    *NAME.lock().unwrap() = name.into();
    fsd::set_unmount(unmount);
    let c_name = CString::new(name).unwrap_or_default();
    let dpp = fsd::dispatch();
    // SAFETY: the framework's functions, with statics of this program.
    unsafe {
        iofunc_func_init(9, &raw mut RAW_CONNECT, 33, &raw mut RAW_IO);
        (*(&raw mut RAW_CONNECT)).funcs[C_OPEN] = Some(raw_open);
        let raw_io = &mut *(&raw mut RAW_IO);
        raw_io.funcs[F_READ] = Some(raw_read);
        raw_io.funcs[F_WRITE] = Some(raw_write);
        raw_io.funcs[F_DEVCTL] = Some(raw_devctl);
        raw_io.funcs[F_CLOSE_OCB] = Some(raw_close);
        iofunc_attr_init(&raw mut DEVICE, 0o020666, null(), null());
        let attr = ResmgrAttr { flags: 0, nparts_max: 1, msg_max_size: 8192, other_func: None, reserved: [0; 4] };
        let id = resmgr_attach(
            dpp,
            &attr,
            c_name.as_ptr().cast::<c_char>(),
            0,
            0,
            &raw const RAW_CONNECT,
            &raw const RAW_IO,
            (&raw mut DEVICE).cast::<c_void>(),
        );
        if id < 0 {
            eprintln!("diskd: can't attach {name}: {}", std::io::Error::last_os_error());
            std::process::exit(1);
        }
    }
    if let Some(path) = path
        && let Err(err) = mount(path)
    {
        eprintln!("diskd: can't mount {path}: error {err}");
        std::process::exit(1);
    }
    fsd::serve()
}

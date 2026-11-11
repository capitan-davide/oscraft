//! More calls of one message each: `statfs()`, the nice values, the
//! POSIX scheduling calls by pid, `sigaltstack()`, `shm_open2()` and
//! `getlogin()`.

use core::ffi::{c_char, c_int};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use proto::errno::{EINVAL, ENAMETOOLONG, ERANGE};

use crate::errno::{fail, last, set_last};
use crate::kercall;
use crate::tty::{DEVDIR_FROM, dcmd, devctl};

const BSD_O_RDONLY: c_int = 0;
const BSD_O_CLOEXEC: c_int = 0x10_0000;

// ---- statfs ----

/// QNX's `struct __msg_statvfs`, the payload of `DCMD_FSYS_STATVFS`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct MsgStatvfs {
    bsize: u32,
    frsize: u32,
    blocks: u64,
    bfree: u64,
    bavail: u64,
    files: u64,
    ffree: u64,
    favail: u64,
    fsid: u32,
    basetype: [u8; 16],
    flag: u32,
    namemax: u32,
    filler: [u32; 21],
}

const _: () = assert!(size_of::<MsgStatvfs>() == 168);

/// FreeBSD's `struct statfs` (`<sys/mount.h>`), 2344 bytes.
#[repr(C)]
pub struct Statfs {
    version: u32,
    r#type: u32,
    flags: u64,
    bsize: u64,
    iosize: u64,
    blocks: u64,
    bfree: u64,
    bavail: i64,
    files: u64,
    ffree: i64,
    syncwrites: u64,
    asyncwrites: u64,
    syncreads: u64,
    asyncreads: u64,
    nvnodelistsize: u32,
    spare0: u32,
    spare: [u64; 9],
    namemax: u32,
    owner: u32,
    fsid: [i32; 2],
    charspare: [u8; 80],
    fstypename: [u8; 16],
    mntfromname: [u8; 1024],
    mntonname: [u8; 1024],
}

const _: () = assert!(size_of::<Statfs>() == 2344);
const STATFS_VERSION: u32 = 0x2014_0518;

impl Statfs {
    /// Says where the file system is mounted, from `getfsstat()`.
    pub(crate) fn name(&mut self, from: &[u8], on: &[u8], kind: &[u8]) {
        self.version = STATFS_VERSION;
        let copy = |to: &mut [u8], b: &[u8]| {
            let n = b.len().min(to.len() - 1);
            to[..n].copy_from_slice(&b[..n]);
            to[n] = 0;
        };
        copy(&mut self.mntfromname, from);
        copy(&mut self.mntonname, on);
        copy(&mut self.fstypename, kind);
    }
}

/// `fstatfs(fd, buf)`: `DCMD_FSYS_STATVFS`, which the file systems
/// answer (the kernel's built-in servers don't: their error).
///
/// # Safety
///
/// `buf` is a `struct statfs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_fstatfs(fd: c_int, buf: *mut Statfs) -> c_int {
    let mut v = MsgStatvfs::default();
    // SAFETY: the payload is on this stack, of the command's size.
    if let Err(e) = unsafe { devctl(fd, dcmd(DEVDIR_FROM, 2, 13, size_of::<MsgStatvfs>()), (&raw mut v).cast(), size_of::<MsgStatvfs>()) } {
        return fail(e) as c_int;
    }
    // SAFETY: the caller's guarantee; a zeroed struct is valid.
    unsafe { buf.write_bytes(0, 1) };
    // SAFETY: as above.
    let out = unsafe { &mut *buf };
    out.version = STATFS_VERSION;
    out.bsize = u64::from(if v.frsize != 0 { v.frsize } else { v.bsize });
    out.iosize = u64::from(v.bsize);
    out.blocks = v.blocks;
    out.bfree = v.bfree;
    out.bavail = v.bavail as i64;
    out.files = v.files;
    out.ffree = v.ffree as i64;
    out.namemax = v.namemax;
    out.fsid = [v.fsid as i32, 0];
    out.fstypename = v.basetype;
    out.fstypename[15] = 0;
    // A native file system also says where it is: `from` NUL `on`.
    let mut names = [0u8; 2048];
    // SAFETY: the array, of that size.
    if crate::fd::native(fd) && unsafe { devctl(fd, dcmd(DEVDIR_FROM, b'M' as u32, 3, 2048), names.as_mut_ptr().cast(), 2048) }.is_ok() {
        let mut parts = names.split(|&c| c == 0);
        if let (Some(from), Some(on)) = (parts.next(), parts.next()) {
            let kind = out.fstypename;
            let end = kind.iter().position(|&c| c == 0).unwrap_or(0);
            out.name(from, on, &kind[..end]);
        }
    }
    0
}

/// `statfs(path, buf)`: on an open of the path.
///
/// # Safety
///
/// `path` is a string; `buf` a `struct statfs`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_statfs(path: *const c_char, buf: *mut Statfs) -> c_int {
    // SAFETY: the caller's guarantee.
    let fd = unsafe { crate::fs::__sys_open(path, BSD_O_RDONLY | BSD_O_CLOEXEC, 0) };
    if fd < 0 {
        return -1;
    }
    // SAFETY: as above.
    let r = unsafe { __sys_fstatfs(fd, buf) };
    let e = last();
    let _ = crate::fs::__sys_close(fd);
    set_last(e);
    r
}

// ---- nice values: QNX's nice() does nothing; here a nice value moves
// the priority below (or above) the default, 10 ----

const PRIO_PROCESS: c_int = 0;
const DEFAULT_PRIORITY: i32 = 10;

/// `getpriority(which, who)`: 10 less the base priority of the process
/// (`who` 0: the caller); only `PRIO_PROCESS`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_getpriority(which: c_int, who: c_int) -> c_int {
    if which != PRIO_PROCESS {
        return fail(EINVAL) as c_int;
    }
    let mut param = [0u8; 48];
    match kercall::sched_get(who, 0, &mut param) {
        Ok(_) => (DEFAULT_PRIORITY - i32::from_le_bytes([param[0], param[1], param[2], param[3]])).clamp(-20, 20),
        Err(e) => fail(e) as c_int,
    }
}

/// `setpriority(which, who, nice)`: the priority becomes 10 less the
/// nice value, the policy unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_setpriority(which: c_int, who: c_int, nice: c_int) -> c_int {
    const SCHED_NOCHANGE: u32 = 0;
    if which != PRIO_PROCESS {
        return fail(EINVAL) as c_int;
    }
    let mut param = [0u8; 48];
    let prio = (DEFAULT_PRIORITY - nice.clamp(-20, 20)).max(1);
    param[0..4].copy_from_slice(&prio.to_le_bytes());
    kercall::sched_set(who, 0, SCHED_NOCHANGE, &param).map_or_else(|e| fail(e) as c_int, |()| 0)
}

// ---- the POSIX scheduling calls, by pid: the rtprio mapping ----

const BSD_SCHED_FIFO: c_int = 1;
const BSD_SCHED_OTHER: c_int = 2;
const BSD_SCHED_RR: c_int = 3;
const QNX_SCHED_FIFO: u32 = 1;
const QNX_SCHED_RR: u32 = 2;
const QNX_SCHED_OTHER: u32 = 3;
/// The real-time policies run at QNX 11 + sched_priority (`thread.rs`).
const RT_BASE: i32 = 11;
const RTP_PRIO_MAX: i32 = 31;

#[unsafe(no_mangle)]
pub extern "C" fn __sys_sched_getscheduler(pid: c_int) -> c_int {
    let mut param = [0u8; 48];
    match kercall::sched_get(pid, 0, &mut param) {
        Ok(QNX_SCHED_FIFO) => BSD_SCHED_FIFO,
        Ok(QNX_SCHED_RR) if i32::from_le_bytes([param[0], param[1], param[2], param[3]]) >= RT_BASE => BSD_SCHED_RR,
        Ok(_) => BSD_SCHED_OTHER,
        Err(e) => fail(e) as c_int,
    }
}

/// `sched_getparam(pid, param)`: the POSIX priority, 0 for a
/// time-sharing process.
///
/// # Safety
///
/// `param` is a `struct sched_param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sched_getparam(pid: c_int, param: *mut c_int) -> c_int {
    let mut p = [0u8; 48];
    match kercall::sched_get(pid, 0, &mut p) {
        Ok(policy) => {
            let base = i32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            let prio = if matches!(policy, QNX_SCHED_FIFO | QNX_SCHED_RR) && base >= RT_BASE { (base - RT_BASE).min(RTP_PRIO_MAX) } else { 0 };
            // SAFETY: the caller's guarantee.
            unsafe { param.write(prio) };
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `sched_setscheduler(pid, policy, param)`: the old policy.
///
/// # Safety
///
/// `param` is a `struct sched_param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sched_setscheduler(pid: c_int, policy: c_int, param: *const c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let prio = unsafe { param.read() };
    let (qpolicy, qprio) = match policy {
        BSD_SCHED_FIFO | BSD_SCHED_RR if (0..=RTP_PRIO_MAX).contains(&prio) => {
            (if policy == BSD_SCHED_FIFO { QNX_SCHED_FIFO } else { QNX_SCHED_RR }, RT_BASE + prio)
        }
        BSD_SCHED_OTHER if prio == 0 => (QNX_SCHED_OTHER, DEFAULT_PRIORITY),
        _ => return fail(EINVAL) as c_int,
    };
    let old = __sys_sched_getscheduler(pid);
    if old < 0 {
        return -1;
    }
    let mut p = [0u8; 48];
    p[0..4].copy_from_slice(&qprio.to_le_bytes());
    kercall::sched_set(pid, 0, qpolicy, &p).map_or_else(|e| fail(e) as c_int, |()| old)
}

/// `sched_setparam(pid, param)`: the priority, the policy kept.
///
/// # Safety
///
/// `param` is a `struct sched_param`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sched_setparam(pid: c_int, param: *const c_int) -> c_int {
    let policy = __sys_sched_getscheduler(pid);
    if policy < 0 {
        return -1;
    }
    // SAFETY: the caller's guarantee.
    if unsafe { __sys_sched_setscheduler(pid, policy, param) } < 0 { -1 } else { 0 }
}

// ---- sigaltstack: kept, not used (handlers run on the thread's stack) ----

static ALT_SP: AtomicU64 = AtomicU64::new(0);
static ALT_SIZE: AtomicU64 = AtomicU64::new(0);
static ALT_FLAGS: AtomicU32 = AtomicU32::new(SS_DISABLE);
const SS_DISABLE: u32 = 4;
const MINSIGSTKSZ: u64 = 4096;

/// FreeBSD's `stack_t`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StackT {
    pub sp: u64,
    pub size: u64,
    pub flags: c_int,
    pub pad: c_int,
}

/// `sigaltstack(ss, oss)`: the setting is kept for `oss` to report; the
/// kernel runs handlers on the thread's own stack.
///
/// # Safety
///
/// `ss` and `oss` are `stack_t`s or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigaltstack(ss: *const StackT, oss: *mut StackT) -> c_int {
    if !oss.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe {
            oss.write(StackT {
                sp: ALT_SP.load(Ordering::Relaxed),
                size: ALT_SIZE.load(Ordering::Relaxed),
                flags: ALT_FLAGS.load(Ordering::Relaxed) as c_int,
                pad: 0,
            })
        };
    }
    if !ss.is_null() {
        // SAFETY: the caller's guarantee.
        let s = unsafe { *ss };
        if s.flags as u32 & SS_DISABLE == 0 && s.size < MINSIGSTKSZ {
            return fail(proto::errno::ENOMEM) as c_int;
        }
        ALT_SP.store(s.sp, Ordering::Relaxed);
        ALT_SIZE.store(s.size, Ordering::Relaxed);
        ALT_FLAGS.store(s.flags as u32 & SS_DISABLE, Ordering::Relaxed);
    }
    0
}

// ---- shared memory objects: files of /dev/shmem ----

/// `/dev/shmem/<name>` for a shared memory object's `name`
/// (`SHM_ANON`, (char *)1: a name of this process's own).
///
/// # Safety
///
/// `name` is a string, or `SHM_ANON`.
unsafe fn shm_path(name: *const c_char, out: &mut [u8; 256]) -> Result<(), i32> {
    static ANON: AtomicU32 = AtomicU32::new(0);
    let dir = b"/dev/shmem/";
    out[..dir.len()].copy_from_slice(dir);
    let mut n = dir.len();
    if name as usize == 1 {
        let (pid, seq) = (crate::proc::pid() as u32, ANON.fetch_add(1, Ordering::Relaxed));
        for (label, v) in [(&b"anon-"[..], pid), (&b"-"[..], seq)] {
            out[n..n + label.len()].copy_from_slice(label);
            n += label.len();
            let mut digits = [0u8; 10];
            let (mut v, mut count) = (v, 0);
            while v != 0 || count == 0 {
                digits[count] = b'0' + (v % 10) as u8;
                v /= 10;
                count += 1;
            }
            for i in (0..count).rev() {
                out[n] = digits[i];
                n += 1;
            }
        }
        out[n] = 0;
        return Ok(());
    }
    let mut i = 0;
    // SAFETY: a string, the caller's guarantee.
    while unsafe { name.add(i).read() } == b'/' as c_char {
        i += 1;
    }
    loop {
        // SAFETY: as above.
        let c = unsafe { name.add(i).read() } as u8;
        if n >= out.len() - 1 {
            return Err(ENAMETOOLONG);
        }
        out[n] = c;
        n += 1;
        if c == 0 {
            break;
        }
        i += 1;
    }
    Ok(())
}

/// `shm_open2(path, flags, mode, shmflags, name)`: an open of the file
/// in `/dev/shmem`; the shmflags (large pages, sealing) are not kept.
///
/// # Safety
///
/// `path` is a string or `SHM_ANON`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_shm_open2(path: *const c_char, flags: c_int, mode: u32, _shmflags: c_int, _name: *const c_char) -> c_int {
    let mut full = [0u8; 256];
    // SAFETY: the caller's guarantee.
    if let Err(e) = unsafe { shm_path(path, &mut full) } {
        return fail(e) as c_int;
    }
    let flags = if path as usize == 1 { flags | 0x200 | 0x800 } else { flags }; // O_CREAT | O_EXCL
    // SAFETY: a string on this stack.
    unsafe { crate::fs::__sys_open(full.as_ptr().cast(), flags | BSD_O_CLOEXEC, mode) }
}

/// `shm_unlink(path)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_shm_unlink(path: *const c_char) -> c_int {
    let mut full = [0u8; 256];
    // SAFETY: the caller's guarantee.
    if let Err(e) = unsafe { shm_path(path, &mut full) } {
        return fail(e) as c_int;
    }
    // SAFETY: a string on this stack.
    unsafe { crate::fs::__sys_unlinkat(-100, full.as_ptr().cast(), 0) }
}

/// `getlogin(buf, len)`: the user's name is not kept by the kernel:
/// `root` for uid 0, else none (ERANGE for no room).
///
/// # Safety
///
/// `buf` is writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getlogin(buf: *mut c_char, len: u32) -> c_int {
    let name: &[u8] = if crate::cred::__sys_getuid() == 0 { b"root\0" } else { b"\0" };
    if (len as usize) < name.len() {
        return fail(ERANGE) as c_int;
    }
    // SAFETY: the caller's guarantee.
    unsafe { buf.cast::<u8>().copy_from_nonoverlapping(name.as_ptr(), name.len()) };
    0
}

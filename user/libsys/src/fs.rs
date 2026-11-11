//! The calls that name a path: `open()`, `stat()`, `readlink()`,
//! `access()`, each a connect request (`path`); and `close()`, `lseek()`.
//! FreeBSD's flags come in; QNX's go into the messages.

use core::ffi::{c_char, c_int};

use proto::connect::{CONNECT_COMBINE_CLOSE, CONNECT_MKNOD, CONNECT_OPEN, CONNECT_READLINK, CONNECT_UNLINK, LinkReply};
use proto::errno::{EACCES, EBADF, EBUSY, EINTR, EINVAL, ENAMETOOLONG, ENXIO, EPERM, ETXTBSY};
use proto::fcntl::{self as q, SH_DENYNO};
use proto::io::{self as m, Header, Lseek};
use proto::pod::Pod;
use proto::stat::{S_IFDIR, S_IFLNK, S_IFMT};

use crate::errno::fail;
use crate::kercall::{Iov, SYSMGR_COID, ask, connect_detach, msg_sendv};
use crate::path::{self, PATH_MAX, Request};
use crate::stat::Stat;

// FreeBSD's open flags (`<sys/fcntl.h>`).
const BSD_O_ACCMODE: c_int = 3;
const BSD_O_NONBLOCK: c_int = 0x4;
const BSD_O_APPEND: c_int = 0x8;
const BSD_O_SYNC: c_int = 0x80;
const BSD_O_NOFOLLOW: c_int = 0x100;
const BSD_O_CREAT: c_int = 0x200;
const BSD_O_TRUNC: c_int = 0x400;
const BSD_O_EXCL: c_int = 0x800;
const BSD_O_NOCTTY: c_int = 0x8000;
const BSD_O_DIRECTORY: c_int = 0x2_0000;
const BSD_O_EXEC: c_int = 0x4_0000;
const BSD_O_CLOEXEC: c_int = 0x10_0000;
const BSD_O_DSYNC: c_int = 0x100_0000;
const BSD_AT_FDCWD: c_int = -100;
const BSD_AT_EACCESS: c_int = 0x100;
const BSD_AT_SYMLINK_NOFOLLOW: c_int = 0x200;

/// The flags for FreeBSD's `oflag`, and the mode bit that asks the server
/// for a link itself (the protocol's way of `O_NOFOLLOW`).
fn qnx_oflag(oflag: c_int) -> (u32, u32) {
    let q = oflag as u32 & !q::O_NOFOLLOW;
    (q | q::O_LARGEFILE, if oflag & BSD_O_NOFOLLOW != 0 { S_IFLNK } else { 0 })
}

/// The directory field for a `*at()` request: `None` for the working
/// directory or an absolute path.
///
/// # Safety
///
/// `path` is a string.
unsafe fn dir_of(dirfd: c_int, path: *const c_char) -> Result<Option<u16>, c_int> {
    // SAFETY: the caller's guarantee.
    if dirfd == BSD_AT_FDCWD || unsafe { path.read() } == b'/' as c_char {
        return Ok(None);
    }
    u16::try_from(dirfd).map(Some).map_err(|_| EBADF)
}

/// `openat(dirfd, path, oflag, mode)`: the connection to the file's
/// server is the descriptor.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_openat(dirfd: c_int, path: *const c_char, oflag: c_int, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let (ioflag, link) = qnx_oflag(oflag);
    let request = Request {
        subtype: CONNECT_OPEN,
        file_type: 0,
        ioflag,
        mode: (mode & !S_IFMT) | link,
        sflag: SH_DENYNO,
        access: 7,
        dirfd: dir,
        extra_type: 0,
        extra: &[],
        base: 0,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => c.coid,
        Err(e) => fail(e) as c_int,
    }
}

/// `open(path, oflag, mode)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_open(path: *const c_char, oflag: c_int, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_openat(BSD_AT_FDCWD, path, oflag, mode) }
}

/// `close(fd)`: `_IO_CLOSE` to the server, then the connection detached.
/// If the server's answer is interrupted the descriptor is left open, so
/// that the close can be tried again.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_close(fd: c_int) -> c_int {
    crate::logsock::closing(fd);
    let message = Header {
        r#type: m::IO_CLOSE,
        combine_len: size_of::<Header>() as u16,
    };
    let send = [Iov::of(&message)];
    // SAFETY: the message is on this stack.
    let sent = unsafe { msg_sendv(fd, &send, &[]) };
    if let Err(EINTR) = sent {
        return fail(EINTR) as c_int;
    }
    let detached = connect_detach(fd);
    match (sent, detached) {
        (Err(e), _) | (_, Err(e)) => fail(e) as c_int,
        _ => 0,
    }
}

/// `lseek(fd, offset, whence)`: `_IO_LSEEK`, the new offset replied.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_lseek(fd: c_int, offset: i64, whence: c_int) -> i64 {
    let message = Lseek {
        r#type: m::IO_LSEEK,
        combine_len: size_of::<Lseek>() as u16,
        whence: whence as i16,
        flags: 0,
        offset: offset as u64,
    };
    match ask::<Lseek, u64>(fd, &message) {
        Ok((_, result)) => result as i64,
        Err(e) => fail(e) as i64,
    }
}

/// `fstatat(dirfd, path, sb, flag)`: a connect request combined with
/// `_IO_STAT`, closed at once, on a side connection.
///
/// # Safety
///
/// `path` is a string; `sb` points to a FreeBSD `struct stat`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_fstatat(dirfd: c_int, path: *const c_char, sb: *mut Stat, flag: c_int) -> c_int {
    if flag & !BSD_AT_SYMLINK_NOFOLLOW != 0 {
        return fail(EINVAL) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let message = m::Stat {
        r#type: m::IO_STAT,
        combine_len: size_of::<m::Stat>() as u16,
        format: proto::stat::FORM_PREFERRED,
    };
    let mode = if flag & BSD_AT_SYMLINK_NOFOLLOW != 0 { S_IFLNK } else { 0 };
    let mut buffer: crate::stat::Reply = [0; size_of::<Stat>()];
    // SAFETY: the caller's guarantee; the reply is a stat.
    let form = match unsafe { path::probe(dir, mode, message.as_bytes(), path, &mut buffer) } {
        Ok(form) => form as u32,
        Err(e) => return fail(e) as c_int,
    };
    // SAFETY: the caller's guarantee.
    unsafe { sb.write(crate::stat::decode(form, &buffer)) };
    0
}

/// `readlinkat(dirfd, path, buf, size)`: `_IO_CONNECT_READLINK`; the
/// reply is a link reply's header and the target.
///
/// # Safety
///
/// `path` is a string; `buf` is writable for `size` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_readlinkat(dirfd: c_int, path: *const c_char, buf: *mut c_char, size: usize) -> isize {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    let request = Request {
        subtype: CONNECT_READLINK,
        file_type: 0,
        ioflag: q::O_LARGEFILE | q::O_NOCTTY,
        mode: S_IFLNK,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: dir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    let mut reply = [0u8; size_of::<LinkReply>() + PATH_MAX];
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut reply) } {
        Ok(c) => c.detach(),
        Err(e) => return fail(e),
    }
    let target = &reply[size_of::<LinkReply>()..];
    let header = LinkReply::read_from(&reply).unwrap_or_default();
    let len = match usize::from(header.path_len) {
        0 => target.iter().position(|&c| c == 0).unwrap_or(PATH_MAX),
        n => n,
    };
    if len >= PATH_MAX {
        return fail(ENAMETOOLONG);
    }
    // The server's length counts the NUL, which isn't the link's.
    let len = if len > 0 && target[len - 1] == 0 { len - 1 } else { len };
    let n = len.min(size);
    // SAFETY: the caller's guarantee: buf holds size bytes.
    unsafe { core::ptr::copy_nonoverlapping(target.as_ptr().cast(), buf, n) };
    n as isize
}

/// `readlink(path, buf, size)`.
///
/// # Safety
///
/// As `readlinkat()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_readlink(path: *const c_char, buf: *mut c_char, size: usize) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_readlinkat(BSD_AT_FDCWD, path, buf, size) }
}

/// `faccessat(dirfd, path, amode, flag)`: an open that only checks
/// permissions (`O_PERMCHECK`), as the real IDs unless `AT_EACCESS`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_faccessat(dirfd: c_int, path: *const c_char, amode: c_int, flag: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    if !(0..=7).contains(&amode) {
        return fail(EINVAL) as c_int;
    }
    // R 4, W 2, X 1 become _IO_FLAG_RD 1, _WR 2, _EXEC 4, less one, in
    // O_ACCMODE's bits (the form a connect request takes).
    let wanted = u32::from(amode & 4 != 0) | (u32::from(amode & 2 != 0) << 1) | (u32::from(amode & 1 != 0) << 2);
    let ids = if flag & BSD_AT_EACCESS != 0 { 0 } else { q::O_REALIDS };
    let request = Request {
        subtype: CONNECT_COMBINE_CLOSE,
        file_type: 0,
        ioflag: q::O_LARGEFILE | q::O_NOCTTY | q::O_NONBLOCK | q::O_PERMCHECK | ids | (wanted.wrapping_sub(1) & 7),
        mode: 0,
        sflag: SH_DENYNO,
        access: 7,
        dirfd: dir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => {
            c.detach();
            0
        }
        // Busy, or not openable now: but it exists and may be accessed.
        Err(ENXIO | EBUSY | ETXTBSY) => 0,
        Err(EPERM) => fail(EACCES) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

/// `access(path, amode)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_access(path: *const c_char, amode: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_faccessat(BSD_AT_FDCWD, path, amode, 0) }
}

/// `eaccess(path, amode)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_eaccess(path: *const c_char, amode: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_faccessat(BSD_AT_FDCWD, path, amode, BSD_AT_EACCESS) }
}


const BSD_AT_REMOVEDIR: c_int = 0x800;

/// A request that only acts on a path (unlink, make a node): on a side
/// connection, closed at once; the server's status is the result.
///
/// # Safety
///
/// `path` is a string.
unsafe fn act(dirfd: c_int, path: *const c_char, subtype: u16, ioflag: u32, mode: u32, file_type: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    let dir = match unsafe { dir_of(dirfd, path) } {
        Ok(d) => d,
        Err(e) => return fail(e) as c_int,
    };
    let request = Request {
        subtype,
        file_type,
        ioflag,
        mode,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: dir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect(&request, path, &mut []) } {
        Ok(c) => {
            c.detach();
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `unlinkat(dirfd, path, flag)`: a file, or with `AT_REMOVEDIR` an
/// empty directory. The mode says what may be removed.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_unlinkat(dirfd: c_int, path: *const c_char, flag: c_int) -> c_int {
    if flag & !BSD_AT_REMOVEDIR != 0 {
        return fail(EINVAL) as c_int;
    }
    let mode = if flag & BSD_AT_REMOVEDIR != 0 { S_IFDIR } else { S_IFLNK };
    // SAFETY: the caller's guarantee.
    unsafe { act(dirfd, path, CONNECT_UNLINK, q::O_NOCTTY, mode, 0) }
}

/// `unlink(path)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_unlink(path: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_unlinkat(BSD_AT_FDCWD, path, 0) }
}

/// `rmdir(path)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_rmdir(path: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_unlinkat(BSD_AT_FDCWD, path, BSD_AT_REMOVEDIR) }
}

/// `mkdirat(dirfd, path, mode)`: a node of type directory.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mkdirat(dirfd: c_int, path: *const c_char, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { act(dirfd, path, CONNECT_MKNOD, q::O_NOCTTY | q::O_CREAT, (mode & !S_IFMT) | S_IFDIR, 0) }
}

/// `mkdir(path, mode)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_mkdir(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_mkdirat(BSD_AT_FDCWD, path, mode) }
}

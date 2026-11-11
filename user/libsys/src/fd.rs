//! The rest of what a shell asks of its descriptors and its process:
//! pipes, duplication and `fcntl()`, directory entries, the working
//! directory, vectored and positioned I/O, truncation and sync, modes,
//! the mask, process groups and sessions, sleeping and waiting for a
//! signal. Each is the message QNX's libc sends.

use core::ffi::{c_char, c_int, c_void};

use proto::connect::{CONNECT_COMBINE_CLOSE, CONNECT_OPEN, Connect, FTYPE_PIPE, PATH_CHDIR};
use proto::errno::{EACCES, EBADF, EINTR, EINVAL, ENAMETOOLONG, ENOENT, ENOSYS, ENOTDIR, ERANGE, ESPIPE};
use proto::fcntl::{self as q, SH_DENYNO};
use proto::info::MsgInfoHdr;
use proto::io::{self as m, Chmod, Devctl, DevctlReply, Dup, Openfd, ReadWrite, Space, Sync};
use proto::pod::Pod;
use proto::proc::{self as p, GetSetId, Session, SetPgid, Umask, UmaskReply};
use proto::stat::{S_IFDIR, S_IFMT};

use crate::errno::{fail, not_implemented};
use crate::kercall::{self, Iov, SYSMGR_COID, ask, connect_attach, connect_detach, msg_sendv, send};
use crate::path::{self, PATH_MAX, Request};

const BSD_AT_FDCWD: c_int = -100;
const BSD_O_NONBLOCK: c_int = 0x4;
const BSD_O_APPEND: c_int = 0x8;
const BSD_O_SYNC: c_int = 0x80;
const BSD_O_NOCTTY: c_int = 0x8000;
const BSD_O_CLOEXEC: c_int = 0x10_0000;
const BSD_O_DSYNC: c_int = 0x100_0000;
/// `_NTO_COF_*`: a connection's flags.
const COF_CLOEXEC: c_int = 0x1;
const COF_NOSHARE: u16 = 0x40;
const COF_PIPE: u16 = 0x400;
const COF_DUP2: c_int = 0x8000;
/// `_NTO_SIDE_CHANNEL`.
const SIDE_CHANNEL: c_int = 0x4000_0000;
const FD_CLOEXEC: u32 = 1;

/// `struct _server_info` (`<sys/neutrino.h>`): who serves a connection.
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub(crate) struct ServerInfo {
    pub nd: u32,
    pub srcnd: u32,
    pub pid: i32,
    pub tid: i32,
    pub chid: i32,
    pub scoid: i32,
    pub coid: i32,
    pub priority: i16,
    pub flags: u16,
    pub rest: [u32; 16],
}

pub(crate) fn server_info(fd: c_int) -> Result<ServerInfo, c_int> {
    let mut info = ServerInfo::default();
    if fd < 0 || fd & SIDE_CHANNEL != 0 {
        return Err(EBADF);
    }
    // SAFETY: the info is on this stack.
    if unsafe { kercall::connect_server_info(fd, (&raw mut info).cast()) }? != fd {
        return Err(EBADF);
    }
    Ok(info)
}

/// Whether `fd`'s server speaks FreeBSD's layouts: anything but a QNX
/// binary, which the kernel says in `reserved`, the last word of
/// `struct _server_info`.
pub(crate) fn native(fd: c_int) -> bool {
    const RESERVED: usize = 7;
    server_info(fd).is_ok_and(|i| i.rest[RESERVED] == 0)
}

/// The client side of `struct _msg_info_hdr` that `_IO_DUP` and
/// `_IO_OPENFD` send: this process and its connection `coid`.
pub(crate) fn client_of(info: &ServerInfo, coid: c_int) -> MsgInfoHdr {
    MsgInfoHdr {
        pid: crate::proc::pid(),
        chid: info.chid,
        scoid: info.scoid,
        coid,
        ..MsgInfoHdr::default()
    }
}

/// Duplicates `fd`: a new connection to the same server, as the lowest
/// descriptor from `min`, or as exactly `target` (closing what it was);
/// the server learns of it with `_IO_DUP`.
fn dup(fd: c_int, min: c_int, target: Option<c_int>, cloexec: bool) -> Result<c_int, c_int> {
    let info = server_info(fd)?;
    let flags = c_int::from(info.flags & COF_NOSHARE) | COF_CLOEXEC;
    let new = match target {
        Some(t) if t == fd => return Ok(fd),
        Some(t) => {
            let close = m::Header {
                r#type: m::IO_CLOSE,
                combine_len: size_of::<m::Header>() as u16,
            };
            if let Err(EINTR) = send(t, &close) {
                return Err(EINTR);
            }
            connect_attach(0, 0, fd, t as u32, flags | COF_DUP2)
        }
        None => connect_attach(info.nd, info.pid, info.chid, min as u32, flags),
    }
    .map_err(|_| EBADF)?;
    let msg = Dup {
        r#type: m::IO_DUP,
        combine_len: size_of::<Dup>() as u16,
        info: client_of(&info, fd),
        handle: 0,
    };
    if let Err(e) = send(new, &msg) {
        let _ = connect_detach(new);
        return Err(e);
    }
    if !cloexec {
        let _ = kercall::connect_flags(new, FD_CLOEXEC, 0);
    }
    Ok(new)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_dup(fd: c_int) -> c_int {
    dup(fd, 0, None, false).unwrap_or_else(|e| fail(e) as c_int)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_dup2(fd: c_int, to: c_int) -> c_int {
    if to < 0 {
        return fail(EBADF) as c_int;
    }
    dup(fd, to, Some(to), false).unwrap_or_else(|e| fail(e) as c_int)
}

// FreeBSD's fcntl commands.
const F_DUPFD: c_int = 0;
const F_GETFD: c_int = 1;
const F_SETFD: c_int = 2;
const F_GETFL: c_int = 3;
const F_SETFL: c_int = 4;
const F_GETOWN: c_int = 5;
const F_SETOWN: c_int = 6;
const F_DUP2FD: c_int = 10;
const F_DUPFD_CLOEXEC: c_int = 17;
const F_DUP2FD_CLOEXEC: c_int = 18;
/// libc's `opendir()` asks whether the directory is a union stack: no
/// union mounts here.
const F_ISUNIONSTACK: c_int = 21;
const DEVDIR_TO: u32 = 0x8000_0000;
const DEVDIR_FROM: u32 = 0x4000_0000;
const DCMD_ALL_GETFLAGS: u32 = 0x4004_0101;
const DCMD_ALL_SETFLAGS: u32 = 0x8004_0102;
const DCMD_ALL_GETOWN: u32 = 0x4004_0104;
const DCMD_ALL_SETOWN: u32 = 0x8004_0105;

/// A device command on `fd` with an int in or out: the server's answer.
fn devctl_int(fd: c_int, dcmd: u32, value: &mut c_int) -> Result<c_int, c_int> {
    let msg = Devctl {
        r#type: m::IO_DEVCTL,
        combine_len: size_of::<Devctl>() as u16,
        dcmd: dcmd as i32,
        nbytes: 4,
        zero: 0,
    };
    let mut answer = DevctlReply::default();
    let data = (value as *mut c_int).cast::<c_void>();
    let sendv = [Iov::of(&msg), Iov::raw(data, if dcmd & DEVDIR_TO != 0 { 4 } else { 0 })];
    let recv = [
        Iov::raw((&raw mut answer).cast(), size_of::<DevctlReply>()),
        Iov::raw(data, if dcmd & DEVDIR_FROM != 0 { 4 } else { 0 }),
    ];
    // SAFETY: every part is on this stack or the caller's int.
    unsafe { msg_sendv(fd, &sendv, &recv) }?;
    Ok(answer.ret_val)
}

/// `fcntl(fd, cmd, arg)`: duplication, the close-on-exec flag (the
/// kernel's, on the connection), the status flags and owner (the
/// server's, as device commands). Record locks are not done yet.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_fcntl(fd: c_int, cmd: c_int, arg: isize) -> c_int {
    let a = arg as c_int;
    let r = match cmd {
        F_DUPFD | F_DUPFD_CLOEXEC | F_DUP2FD | F_DUP2FD_CLOEXEC => {
            if a < 0 || a & SIDE_CHANNEL != 0 {
                Err(if cmd == F_DUP2FD || cmd == F_DUP2FD_CLOEXEC { EBADF } else { EINVAL })
            } else {
                let target = (cmd == F_DUP2FD || cmd == F_DUP2FD_CLOEXEC).then_some(a);
                dup(fd, a, target, cmd == F_DUPFD_CLOEXEC || cmd == F_DUP2FD_CLOEXEC)
            }
        }
        F_GETFD => kercall::connect_flags(fd, 0, 0).map(|f| f as c_int & FD_CLOEXEC as c_int),
        F_SETFD => kercall::connect_flags(fd, FD_CLOEXEC, a as u32 & FD_CLOEXEC).map(|_| 0),
        F_GETFL => {
            let mut v = 0;
            devctl_int(fd, DCMD_ALL_GETFLAGS, &mut v)
                .map(|_| if native(fd) { v } else { qnxabi::fcntl::flags_from_qnx(v as u32) as c_int })
        }
        F_SETFL => {
            let mut v = if native(fd) { a } else { qnxabi::fcntl::flags_to_qnx(a as u32) as c_int };
            devctl_int(fd, DCMD_ALL_SETFLAGS, &mut v).map(|_| 0)
        }
        F_GETOWN => {
            let mut v = 0;
            devctl_int(fd, DCMD_ALL_GETOWN, &mut v).map(|_| v)
        }
        F_SETOWN => {
            let mut v = a;
            devctl_int(fd, DCMD_ALL_SETOWN, &mut v).map(|_| 0)
        }
        F_ISUNIONSTACK => Ok(0),
        _ => {
            not_implemented("fcntl command");
            Err(ENOSYS)
        }
    };
    r.unwrap_or_else(|e| fail(e) as c_int)
}

/// `pipe2(fildes, flags)`: the pipe manager is a server of file type
/// `_FTYPE_PIPE`; an anonymous open of "/" for that type makes a pipe,
/// and `_IO_OPENFD` on its connection opens the other end. Each end has
/// a connection of its own (`_NTO_COF_NOSHARE`).
///
/// # Safety
///
/// `fildes` holds two ints.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_pipe2(fildes: *mut c_int, flags: c_int) -> c_int {
    if flags & !(BSD_O_NONBLOCK | BSD_O_CLOEXEC) != 0 {
        return fail(EINVAL) as c_int;
    }
    let mut ioflag = q::O_ANON | q::O_NOCTTY;
    if flags & BSD_O_NONBLOCK != 0 {
        ioflag |= q::O_NONBLOCK;
    }
    if flags & BSD_O_CLOEXEC != 0 {
        ioflag |= q::O_CLOEXEC;
    }
    let request = Request {
        subtype: CONNECT_OPEN,
        file_type: FTYPE_PIPE,
        ioflag,
        mode: 0,
        sflag: SH_DENYNO,
        access: 3,
        dirfd: None,
        extra_type: 0,
        extra: &[],
        base: 0,
        coflags: c_int::from(COF_NOSHARE),
    };
    // SAFETY: a literal.
    let read_end = match unsafe { path::connect(&request, c"/".as_ptr(), &mut []) } {
        Ok(c) => c.coid,
        Err(ENOENT) => return fail(ENOSYS) as c_int, // no pipe manager
        Err(e) => return fail(e) as c_int,
    };
    let info = match server_info(read_end) {
        Ok(i) => i,
        Err(e) => return fail(e) as c_int,
    };
    let _ = info.flags & COF_PIPE;
    // The other end: a connection with the pipe flags, `_IO_OPENFD`.
    let write_end = match connect_attach(info.nd, info.pid, info.chid, 0, 0x41) {
        Ok(c) => c,
        Err(e) => {
            __sys_close_quiet(read_end);
            return fail(e) as c_int;
        }
    };
    let oflag = q::O_WRONLY | if flags & BSD_O_NONBLOCK != 0 { q::O_NONBLOCK } else { 0 };
    let msg = Openfd {
        r#type: m::IO_OPENFD,
        combine_len: size_of::<Openfd>() as u16,
        ioflag: ((oflag + 1) & q::O_ACCMODE) | (oflag & !(q::O_ACCMODE | q::O_CLOEXEC)) | q::O_LARGEFILE,
        sflag: SH_DENYNO,
        xtype: m::IO_OPENFD_PIPE,
        info: client_of(&info, read_end),
        handle: 0,
    };
    if let Err(e) = send(write_end, &msg) {
        let _ = connect_detach(write_end);
        __sys_close_quiet(read_end);
        return fail(e) as c_int;
    }
    if flags & BSD_O_CLOEXEC == 0 {
        let _ = kercall::connect_flags(write_end, FD_CLOEXEC, 0);
    }
    // SAFETY: the caller's guarantee.
    unsafe {
        fildes.write(read_end);
        fildes.add(1).write(write_end);
    }
    0
}

fn __sys_close_quiet(fd: c_int) {
    let _ = crate::fs::__sys_close(fd);
}

/// QNX's `struct dirent`'s header: the name follows its name's length.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct QnxDirent {
    ino: u64,
    offset: i64,
    reclen: i16,
    namelen: i16,
}
// SAFETY: plain data.
unsafe impl Pod for QnxDirent {}
const QNX_NAME_AT: usize = 20;
const QNX_MIN_RECLEN: usize = 0x18;
/// FreeBSD's `struct dirent`'s header: fileno, off, reclen, type, pad,
/// namlen, pad; the name follows at 24.
const BSD_NAME_AT: usize = 24;
const XTYPE_READDIR: u32 = 9;
const XFLAG_STAT_FORM_SHIFT: u32 = 12;
const DT_UNKNOWN: u8 = 0;

/// `getdirentries(fd, buf, count, basep)`: an `_IO_READ` of extended
/// type `_IO_XTYPE_READDIR`, its QNX records laid out again as
/// FreeBSD's. A FreeBSD record is at most 8 bytes longer than QNX's, so
/// three quarters of `count` are asked of the server: what it gives
/// always fits, and nothing it advanced past is lost.
///
/// # Safety
///
/// `buf` is writable for `count` bytes; `basep` is null or a long.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getdirentries(fd: c_int, buf: *mut u8, count: usize, basep: *mut i64) -> isize {
    const SCRATCH: usize = 0x1000;
    if count < BSD_NAME_AT + 8 {
        return fail(EINVAL);
    }
    // Native answers come in FreeBSD's `struct dirent`, as they are; QNX's
    // servers are laid out again below.
    if native(fd) {
        let header = ReadWrite {
            r#type: m::IO_READ,
            combine_len: size_of::<ReadWrite>() as u16,
            nbytes: count.min(u32::MAX as usize) as u32,
            xtype: XTYPE_READDIR,
            nbytes_hi: 0,
        };
        let sendv = [Iov::of(&header)];
        let recv = [Iov::raw(buf.cast(), count)];
        // SAFETY: the header is on this stack; `buf` the caller's.
        return match unsafe { msg_sendv(fd, &sendv, &recv) } {
            Ok(n) => {
                if !basep.is_null() {
                    // SAFETY: the caller's guarantee.
                    unsafe { basep.write(crate::fs::__sys_lseek(fd, 0, 1)) };
                }
                n as isize
            }
            Err(e) => fail(e),
        };
    }
    let want = (count * 3 / 4).min(SCRATCH) & !7;
    let mut scratch = [0u8; SCRATCH];
    let header = ReadWrite {
        r#type: m::IO_READ,
        combine_len: size_of::<ReadWrite>() as u16,
        nbytes: want as u32,
        xtype: XTYPE_READDIR | (0x30 << XFLAG_STAT_FORM_SHIFT),
        nbytes_hi: 0,
    };
    let sendv = [Iov::of(&header)];
    let recv = [Iov::raw(scratch.as_mut_ptr().cast(), want)];
    // SAFETY: both are on this stack.
    let got = match unsafe { msg_sendv(fd, &sendv, &recv) } {
        Ok(n) => (n as usize).min(want),
        Err(e) => return fail(e),
    };
    let (mut at, mut out) = (0usize, 0usize);
    while at + QNX_MIN_RECLEN <= got {
        let h = QnxDirent::read_from(&scratch[at..]).unwrap_or_default();
        let (reclen, namelen) = (h.reclen as usize, h.namelen as usize);
        if reclen < QNX_MIN_RECLEN || at + reclen > got || QNX_NAME_AT + namelen >= reclen {
            break;
        }
        let bsd_len = (BSD_NAME_AT + namelen + 1).next_multiple_of(8);
        if out + bsd_len > count {
            break;
        }
        let mut rec = [0u8; 24];
        rec[0..8].copy_from_slice(&h.ino.to_ne_bytes());
        rec[8..16].copy_from_slice(&h.offset.to_ne_bytes());
        rec[16..18].copy_from_slice(&(bsd_len as u16).to_ne_bytes());
        rec[18] = DT_UNKNOWN;
        rec[20..22].copy_from_slice(&(namelen as u16).to_ne_bytes());
        // SAFETY: `out + bsd_len <= count`: within the caller's buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(rec.as_ptr(), buf.add(out), 24);
            core::ptr::copy_nonoverlapping(scratch.as_ptr().add(at + QNX_NAME_AT), buf.add(out + 24), namelen);
            core::ptr::write_bytes(buf.add(out + 24 + namelen), 0, bsd_len - 24 - namelen);
        }
        out += bsd_len;
        at += reclen;
    }
    if !basep.is_null() {
        // SAFETY: the caller's guarantee.
        unsafe { basep.write(crate::fs::__sys_lseek(fd, 0, 1)) };
    }
    out as isize
}

/// Resolves `path` relative to `dirfd` with a status probe: its stat and
/// its absolute path.
///
/// # Safety
///
/// `path` is a string.
unsafe fn resolve(dirfd: Option<u16>, path: *const c_char, resolved: &mut [u8; PATH_MAX]) -> Result<crate::stat::Stat, c_int> {
    let message = m::Stat {
        r#type: m::IO_STAT,
        combine_len: size_of::<m::Stat>() as u16,
        format: proto::stat::FORM_PREFERRED,
    };
    let request = Request {
        subtype: CONNECT_COMBINE_CLOSE,
        file_type: 0,
        ioflag: q::O_LARGEFILE | q::O_NOCTTY,
        mode: 0,
        sflag: SH_DENYNO,
        access: 0,
        dirfd,
        extra_type: 0,
        extra: message.as_bytes(),
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    let mut buffer: crate::stat::Reply = [0; size_of::<crate::stat::Stat>()];
    // SAFETY: the caller's guarantee.
    let c = unsafe { path::connect_resolved(&request, path, &mut buffer, Some(resolved)) }?;
    let form = c.status as u32;
    c.detach();
    let st = crate::stat::decode(form, &buffer);
    if resolved[0] == 0 {
        return Err(ENAMETOOLONG);
    }
    Ok(st)
}

/// Makes the directory `path`, relative to `dirfd`, the working
/// directory: the path manager keeps its absolute path (`_PATH_CHDIR`).
///
/// # Safety
///
/// `path` is a string.
unsafe fn chdir_at(dirfd: Option<u16>, path: *const c_char) -> c_int {
    let mut resolved = [0u8; PATH_MAX];
    // SAFETY: the caller's guarantee.
    let st = match unsafe { resolve(dirfd, path, &mut resolved) } {
        Ok(st) => st,
        Err(e) => return fail(e) as c_int,
    };
    if u32::from(st.st_mode) & S_IFMT != S_IFDIR {
        return fail(ENOTDIR) as c_int;
    }
    let me = crate::cred::euid_egid();
    if me.0 != 0 {
        let bits = if st.st_uid == me.0 {
            u32::from(st.st_mode) >> 6
        } else if st.st_gid == me.1 {
            u32::from(st.st_mode) >> 3
        } else {
            u32::from(st.st_mode)
        };
        if bits & 1 == 0 {
            return fail(EACCES) as c_int;
        }
    }
    let len = resolved.iter().position(|&c| c == 0).unwrap_or(PATH_MAX - 1) + 1;
    let header = Connect {
        r#type: PATH_CHDIR,
        path_len: len as u16,
        ..Connect::default()
    };
    let parts = [Iov::of(&header), Iov::raw(resolved.as_ptr().cast(), len)];
    // SAFETY: both parts are on this stack.
    match unsafe { msg_sendv(SYSMGR_COID, &parts, &[]) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `chdir(path)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_chdir(path: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { chdir_at(None, path) }
}

/// `fchdir(fd)`: "." relative to the directory open on `fd`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_fchdir(fd: c_int) -> c_int {
    match u16::try_from(fd) {
        // SAFETY: a literal.
        Ok(d) => unsafe { chdir_at(Some(d), c".".as_ptr()) },
        Err(_) => fail(EBADF) as c_int,
    }
}

/// `__getcwd(buf, buflen)`: the working directory's absolute path, from
/// a probe of ".".
///
/// # Safety
///
/// `buf` is writable for `buflen` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys___getcwd(buf: *mut c_char, buflen: usize) -> c_int {
    let mut resolved = [0u8; PATH_MAX];
    // SAFETY: a literal.
    if let Err(e) = unsafe { resolve(None, c".".as_ptr(), &mut resolved) } {
        return fail(e) as c_int;
    }
    let len = resolved.iter().position(|&c| c == 0).unwrap_or(0) + 1;
    if len <= 1 || len > buflen {
        return fail(ERANGE) as c_int;
    }
    // SAFETY: the caller's guarantee: at least `len` bytes.
    unsafe { core::ptr::copy_nonoverlapping(resolved.as_ptr().cast(), buf, len) };
    0
}

/// `writev` and `readv`, `pwritev` and `preadv`, `pwrite` and `pread`:
/// one message, the header (with `_IO_XTYPE_OFFSET` and the offset after
/// it, for the positioned ones) and the caller's parts as they are.
///
/// # Safety
///
/// Each part is valid for its length.
unsafe fn transfer(fd: c_int, write: bool, parts: &[Iov], offset: Option<i64>) -> isize {
    const IOV_MAX: usize = 1024;
    if parts.is_empty() || parts.len() > IOV_MAX {
        return fail(EINVAL);
    }
    let Some(nbytes) = parts.iter().try_fold(0usize, |s, p| s.checked_add(p.len)).filter(|&n| n <= isize::MAX as usize) else {
        return fail(EINVAL);
    };
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct WithOffset {
        header: ReadWrite,
        offset: i64,
    }
    // SAFETY: plain data.
    unsafe impl Pod for WithOffset {}
    let big = nbytes > u32::MAX as usize;
    let header = WithOffset {
        header: ReadWrite {
            r#type: match (write, big) {
                (true, false) => m::IO_WRITE,
                (true, true) => m::IO_WRITE64,
                (false, false) => m::IO_READ,
                (false, true) => m::IO_READ64,
            },
            combine_len: if offset.is_some() { size_of::<WithOffset>() } else { size_of::<ReadWrite>() } as u16,
            nbytes: nbytes as u32,
            xtype: if offset.is_some() { m::IO_XTYPE_OFFSET } else { m::IO_XTYPE_NONE },
            nbytes_hi: (nbytes >> 32) as u32,
        },
        offset: offset.unwrap_or(0),
    };
    let head = Iov::raw((&raw const header).cast(), header.header.combine_len as usize);
    let mut all = [Iov::raw(core::ptr::null(), 0); IOV_MAX + 1];
    all[0] = head;
    all[1..=parts.len()].copy_from_slice(parts);
    // SAFETY: the header is on this stack; the parts the caller's.
    let r = if write {
        unsafe { msg_sendv(fd, &all[..=parts.len()], &[]) }
    } else {
        unsafe { msg_sendv(fd, &all[..1], parts) }
    };
    match r {
        Ok(n) => n as isize,
        Err(e) => fail(e),
    }
}

/// # Safety
///
/// `iov` holds `cnt` parts.
unsafe fn parts<'a>(iov: *const Iov, cnt: u32) -> &'a [Iov] {
    // SAFETY: the caller's guarantee.
    unsafe { core::slice::from_raw_parts(iov, cnt as usize) }
}

/// `writev(fd, iov, cnt)`.
///
/// # Safety
///
/// `iov` holds `cnt` parts valid for reads.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_writev(fd: c_int, iov: *const Iov, cnt: u32) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, true, parts(iov, cnt), None) }
}

/// `readv(fd, iov, cnt)`.
///
/// # Safety
///
/// `iov` holds `cnt` parts valid for writes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_readv(fd: c_int, iov: *const Iov, cnt: u32) -> isize {
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, false, parts(iov, cnt), None) }
}

/// `pwrite(fd, buf, nbytes, offset)`.
///
/// # Safety
///
/// `buf` is valid for reads of `nbytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_pwrite(fd: c_int, buf: *const c_void, nbytes: usize, offset: i64) -> isize {
    if offset < 0 {
        return fail(EINVAL);
    }
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, true, &[Iov::raw(buf, nbytes)], Some(offset)) }
}

/// `pread(fd, buf, nbytes, offset)`.
///
/// # Safety
///
/// `buf` is valid for writes of `nbytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_pread(fd: c_int, buf: *mut c_void, nbytes: usize, offset: i64) -> isize {
    if offset < 0 {
        return fail(EINVAL);
    }
    // SAFETY: the caller's guarantee.
    unsafe { transfer(fd, false, &[Iov::raw(buf, nbytes)], Some(offset)) }
}

/// `_IO_SPACE`'s subtypes: free, allocate or grow a file's space.
const F_GROWSP64: u16 = 0x6d;
const F_ALLOCSP64: u16 = 0x6e;
const F_FREESP64: u16 = 0x6f;

fn space(fd: c_int, subtype: u16, start: u64, len: u64) -> Result<u64, c_int> {
    let msg = Space {
        r#type: m::IO_SPACE,
        combine_len: size_of::<Space>() as u16,
        subtype,
        whence: 0,
        start,
        len,
    };
    ask::<Space, u64>(fd, &msg).map(|(_, size)| size)
}

/// `ftruncate(fd, length)`: frees everything from `length` on, and if
/// the file was shorter, grows it.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_ftruncate(fd: c_int, length: i64) -> c_int {
    let fix = |e| if e == ESPIPE { EINVAL } else { e };
    let r = (|| {
        let length = u64::try_from(length).map_err(|_| EINVAL)?;
        let size = space(fd, F_FREESP64, length, 0).map_err(fix)?;
        if size < length {
            match space(fd, F_GROWSP64, size, length - size) {
                Err(EINVAL) => space(fd, F_ALLOCSP64, size, length - size).map_err(fix)?,
                other => other.map_err(fix)?,
            };
        }
        Ok(())
    })();
    r.map_or_else(|e| fail(e) as c_int, |()| 0)
}

fn sync(fd: c_int, flag: u32) -> c_int {
    let msg = Sync {
        r#type: m::IO_SYNC,
        combine_len: size_of::<Sync>() as u16,
        flag: if native(fd) { flag } else { qnxabi::fcntl::flags_to_qnx(flag) },
    };
    match send(fd, &msg) {
        Ok(_) => 0,
        Err(ENOSYS) => fail(EINVAL) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_fsync(fd: c_int) -> c_int {
    sync(fd, q::O_SYNC)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_fdatasync(fd: c_int) -> c_int {
    sync(fd, q::O_DSYNC)
}

/// `fchmod(fd, mode)`: `_IO_CHMOD`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_fchmod(fd: c_int, mode: u32) -> c_int {
    let msg = Chmod {
        r#type: m::IO_CHMOD,
        combine_len: size_of::<Chmod>() as u16,
        mode,
    };
    send(fd, &msg).map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `fchmodat(dirfd, path, mode, flag)`: a probe combined with `_IO_CHMOD`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_fchmodat(dirfd: c_int, path: *const c_char, mode: u32, flag: c_int) -> c_int {
    const AT_SYMLINK_NOFOLLOW: c_int = 0x200;
    // SAFETY: the caller's guarantee.
    let dir = if dirfd == BSD_AT_FDCWD || unsafe { path.read() } == b'/' as c_char {
        None
    } else {
        match u16::try_from(dirfd) {
            Ok(d) => Some(d),
            Err(_) => return fail(EBADF) as c_int,
        }
    };
    let msg = Chmod {
        r#type: m::IO_CHMOD,
        combine_len: size_of::<Chmod>() as u16,
        mode,
    };
    let link = if flag & AT_SYMLINK_NOFOLLOW != 0 { proto::stat::S_IFLNK } else { 0 };
    // SAFETY: the caller's guarantee.
    match unsafe { path::probe(dir, link, msg.as_bytes(), path, &mut []) } {
        Ok(_) => 0,
        Err(e) => fail(e) as c_int,
    }
}

/// `chmod(path, mode)`.
///
/// # Safety
///
/// `path` is a string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_chmod(path: *const c_char, mode: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_fchmodat(BSD_AT_FDCWD, path, mode, 0) }
}

/// `umask(mask)`: the process manager keeps it; the old one.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_umask(mask: u32) -> u32 {
    let request = Umask {
        r#type: p::PROC_UMASK,
        subtype: p::UMASK_SET,
        umask: mask & 0o777,
        pid: 0,
    };
    match ask::<Umask, UmaskReply>(SYSMGR_COID, &request) {
        Ok((_, r)) => r.umask,
        Err(e) => fail(e) as u32,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_setpgid(pid: c_int, pgid: c_int) -> c_int {
    let request = SetPgid {
        r#type: p::PROC_SETPGID,
        zero: 0,
        pid,
        pgid,
    };
    send(SYSMGR_COID, &request).map_or_else(|e| fail(e) as c_int, |_| 0)
}

/// `getpgid(pid)`: the process manager's IDs of `pid`.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_getpgid(pid: c_int) -> c_int {
    let request = GetSetId {
        r#type: p::PROC_GETSETID,
        subtype: p::ID_GETID_NO_CRED,
        pid,
        ..GetSetId::default()
    };
    match ask::<GetSetId, p::GetSetIdReply>(SYSMGR_COID, &request) {
        Ok((_, r)) => r.pgrp,
        Err(e) => fail(e) as c_int,
    }
}

/// `setsid()`: a session operation of the process manager's.
#[unsafe(no_mangle)]
pub extern "C" fn __sys_setsid() -> c_int {
    let request = Session {
        r#type: p::PROC_SESSION,
        xid: 0,
        sid: 0,
        id: 0,
        event: 0,
    };
    send(SYSMGR_COID, &request).map_or_else(|e| fail(e) as c_int, |sid| sid as c_int)
}

/// `sigsuspend(mask)`: `SignalSuspend()`, which returns EINTR once a
/// signal's handler has run.
///
/// # Safety
///
/// `mask` is a FreeBSD set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_sigsuspend(mask: *const crate::signal::SigSet) -> c_int {
    // SAFETY: the caller's guarantee.
    // SAFETY: the set is on this stack.
    match unsafe { kercall::signal_suspend(mask) } {
        Ok(()) => fail(EINTR) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

/// `nanosleep(req, rem)` and `clock_nanosleep(clock, flags, req, rem)`:
/// `TimerTimeout()` with `_NTO_TIMEOUT_NANOSLEEP`, which sleeps in the
/// call; a signal ends it early with EINTR and the time left.
///
/// # Safety
///
/// `req` is a timespec; `rem` null or one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_clock_nanosleep(clock: c_int, flags: c_int, req: *const [i64; 2], rem: *mut [i64; 2]) -> c_int {
    const TIMER_ABSTIME: c_int = 1;
    let Some(qclock) = crate::time::qnx_clock(clock) else {
        return EINVAL;
    };
    // SAFETY: the caller's guarantee.
    let [sec, nsec] = unsafe { *req };
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return EINVAL;
    }
    let ns = sec as u64 * 1_000_000_000 + nsec as u64;
    let mut qflags = kercall::TIMEOUT_NANOSLEEP;
    if flags & TIMER_ABSTIME != 0 {
        qflags |= kercall::TIMEOUT_ABSTIME;
    }
    let mut left: u64 = 0;
    // SAFETY: the times are on this stack.
    match unsafe { kercall::timer_timeout_left(qclock, qflags, ns, &raw mut left) } {
        Ok(()) => 0,
        Err(EINTR) => {
            if !rem.is_null() && flags & TIMER_ABSTIME == 0 {
                // SAFETY: the caller's guarantee.
                unsafe { rem.write([(left / 1_000_000_000) as i64, (left % 1_000_000_000) as i64]) };
            }
            EINTR
        }
        Err(e) => e,
    }
}

/// # Safety
///
/// As `clock_nanosleep()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_nanosleep(req: *const [i64; 2], rem: *mut [i64; 2]) -> c_int {
    // SAFETY: the caller's guarantee.
    match unsafe { __sys_clock_nanosleep(0, 0, req, rem) } {
        0 => 0,
        e => fail(e) as c_int,
    }
}

/// `getrandom(buf, len, flags)`: bytes from `/dev/random`, QNX's random
/// service (what QNX's libc reads too); the kernel's entropy pool feeds
/// it. The flags (blocking, the non-blocking pool) make no difference.
///
/// # Safety
///
/// `buf` is writable for `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getrandom(buf: *mut c_void, len: usize, _flags: u32) -> isize {
    // SAFETY: a literal.
    let fd = unsafe { crate::fs::__sys_open(c"/dev/random".as_ptr(), 0, 0) };
    if fd < 0 {
        return -1;
    }
    let mut got = 0usize;
    while got < len {
        // SAFETY: the caller's guarantee: `len - got` bytes left at `buf + got`.
        let n = unsafe { crate::io::__sys_read(fd, buf.add(got), len - got) };
        if n <= 0 {
            break;
        }
        got += n as usize;
    }
    let _ = crate::fs::__sys_close(fd);
    if got == 0 && len != 0 { -1 } else { got as isize }
}

/// Renames or links: resolves `old` to its server and its path there,
/// then sends that path with `new`'s request to the same server, which
/// must own `new` too (otherwise EXDEV).
///
/// # Safety
///
/// Both are strings.
unsafe fn two_paths(olddir: Option<u16>, old: *const c_char, newdir: Option<u16>, new: *const c_char, old_mode: u32, subtype: u16, extra_type: u8) -> c_int {
    use proto::connect::CONNECT_COMBINE_CLOSE;
    let mut resolved = [0u8; PATH_MAX];
    let probe = Request {
        subtype: CONNECT_COMBINE_CLOSE,
        file_type: 0,
        ioflag: q::O_LARGEFILE | q::O_NOCTTY,
        mode: old_mode,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: olddir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    let first = match unsafe { path::connect_ex(&probe, old, &mut [], Some(&mut resolved), None) } {
        Ok(c) => c,
        Err(e) => return fail(e) as c_int,
    };
    let (server, prefix) = (first.server, first.prefix_len);
    first.detach();
    let Some(end) = resolved.iter().position(|&c| c == 0).filter(|&n| n > 0) else {
        return fail(ENAMETOOLONG) as c_int;
    };
    let same = path::SameServer { server, subtype, extra_type, extra: &resolved[prefix.min(end)..=end] };
    let request = Request {
        subtype: CONNECT_COMBINE_CLOSE,
        file_type: 0,
        ioflag: q::O_LARGEFILE | q::O_NOCTTY,
        mode: old_mode,
        sflag: SH_DENYNO,
        access: 0,
        dirfd: newdir,
        extra_type: 0,
        extra: &[],
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    match unsafe { path::connect_ex(&request, new, &mut [], None, Some(&same)) } {
        Ok(c) => {
            c.detach();
            0
        }
        Err(e) => fail(e) as c_int,
    }
}

/// `dir_of` for the two-path calls.
///
/// # Safety
///
/// `path` is a string.
unsafe fn dir_for(dirfd: c_int, path: *const c_char) -> Result<Option<u16>, c_int> {
    // SAFETY: the caller's guarantee.
    if dirfd == BSD_AT_FDCWD || unsafe { path.read() } == b'/' as c_char {
        return Ok(None);
    }
    u16::try_from(dirfd).map(Some).map_err(|_| EBADF)
}

/// `renameat(olddirfd, old, newdirfd, new)`: a final symbolic link is
/// renamed, not followed.
///
/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_renameat(olddirfd: c_int, old: *const c_char, newdirfd: c_int, new: *const c_char) -> c_int {
    use proto::connect::{CONNECT_RENAME, EXTRA_RENAME};
    // SAFETY: the caller's guarantee.
    let (olddir, newdir) = match (unsafe { dir_for(olddirfd, old) }, unsafe { dir_for(newdirfd, new) }) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(e), _) | (_, Err(e)) => return fail(e) as c_int,
    };
    // SAFETY: the caller's guarantee.
    unsafe { two_paths(olddir, old, newdir, new, proto::stat::S_IFLNK, CONNECT_RENAME, EXTRA_RENAME) }
}

/// `rename(old, new)`.
///
/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_rename(old: *const c_char, new: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_renameat(BSD_AT_FDCWD, old, BSD_AT_FDCWD, new) }
}

/// `linkat(olddirfd, old, newdirfd, new, flag)`: a hard link, which the
/// server of `old` makes under `new`.
///
/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_linkat(olddirfd: c_int, old: *const c_char, newdirfd: c_int, new: *const c_char, flag: c_int) -> c_int {
    use proto::connect::{CONNECT_LINK, EXTRA_LINK};
    const AT_SYMLINK_FOLLOW: c_int = 0x400;
    // SAFETY: the caller's guarantee.
    let (olddir, newdir) = match (unsafe { dir_for(olddirfd, old) }, unsafe { dir_for(newdirfd, new) }) {
        (Ok(o), Ok(n)) => (o, n),
        (Err(e), _) | (_, Err(e)) => return fail(e) as c_int,
    };
    let mode = if flag & AT_SYMLINK_FOLLOW != 0 { 0 } else { proto::stat::S_IFLNK };
    // SAFETY: the caller's guarantee.
    unsafe { two_paths(olddir, old, newdir, new, mode, CONNECT_LINK, EXTRA_LINK) }
}

/// `link(old, new)`.
///
/// # Safety
///
/// Both are strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_link(old: *const c_char, new: *const c_char) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { __sys_linkat(BSD_AT_FDCWD, old, BSD_AT_FDCWD, new, 0) }
}

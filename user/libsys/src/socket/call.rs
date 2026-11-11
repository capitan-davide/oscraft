//! Making sockets and joining them: `socket()`, `bind()`, `connect()`,
//! `listen()`, `accept()`, and the calls that are one device command.

use core::ffi::{c_int, c_void};

use proto::connect::{CONNECT_OPEN, EXTRA_SOCKET, FTYPE_SOCKET};
use proto::errno::{EAFNOSUPPORT, EBADF, EFAULT, EINVAL, ENOENT, ENOSYS};
use proto::fcntl::{self as q, SH_DENYNO};
use proto::io::{self as m, Openfd};

use super::address::{FsName, Use, fs_name};
use super::request;
use crate::errno::fail;
use crate::fd::{client_of, server_info};
use crate::kercall::{self, Iov, connect_attach, connect_detach, send};
use crate::path::{self, Request};
use crate::tty::devctl;

/// `SOCK_CLOEXEC` in a socket's type (FreeBSD's `<sys/socket.h>`).
const SOCK_CLOEXEC: c_int = 0x1000_0000;
/// `FD_CLOEXEC` in the flags of a connection, and `_NTO_COF_CLOEXEC`.
const FD_CLOEXEC: u32 = 1;
const COF_CLOEXEC: c_int = 1;
/// `_IO_OPENFD_ACCEPT`.
const OPENFD_ACCEPT: u16 = 3;

/// `DCMD_*` of `io-sock`.
const DCMD_GETSOCKNAME: u32 = 0x4010_0600;
const DCMD_BIND: u32 = 0x8010_0601;
const DCMD_GETPEERNAME: u32 = 0x4010_0602;
const DCMD_CONNECT: u32 = 0x8010_0603;
const DCMD_LISTEN: u32 = 0x8004_0604;
const DCMD_SHUTDOWN: u32 = 0x8004_0605;

/// The data of a socket's open.
#[repr(C)]
struct SocketExtra {
    r#type: i32,
    protocol: i32,
}

/// `io_sock_accept_t`: a `_IO_OPENFD`, and what `accept()` adds.
#[repr(C)]
#[derive(Default)]
struct Accept {
    open: Openfd,
    r#type: u32,
    flags: c_int,
    anamelen: u32,
    pad: u32,
}

/// The two kinds of `accept()`.
const ACCEPT1: u32 = 1;
const ACCEPT4: u32 = 2;

/// Makes a socket: a connection to the manager's node of the domain.
pub(crate) fn socket(domain: c_int, kind: c_int, protocol: c_int) -> Result<c_int, c_int> {
    let mut path = [0u8; 32];
    let dir = b"/dev/socket/";
    path[..dir.len()].copy_from_slice(dir);
    let mut n = dir.len();
    if domain < 0 {
        path[n] = b'-';
        n += 1;
    }
    let mut digits = [0u8; 10];
    let mut d = domain.unsigned_abs();
    let mut count = 0;
    while d != 0 || count == 0 {
        digits[count] = b'0' + (d % 10) as u8;
        d /= 10;
        count += 1;
    }
    for i in (0..count).rev() {
        path[n] = digits[i];
        n += 1;
    }
    let extra = SocketExtra {
        r#type: kind & !SOCK_CLOEXEC,
        protocol,
    };
    // SAFETY: the extra data is a plain structure of its size.
    let bytes = unsafe { core::slice::from_raw_parts((&raw const extra).cast::<u8>(), size_of::<SocketExtra>()) };
    let request = Request {
        subtype: CONNECT_OPEN,
        file_type: FTYPE_SOCKET,
        ioflag: q::O_RDWR | if kind & SOCK_CLOEXEC != 0 { q::O_CLOEXEC } else { 0 },
        mode: 0,
        sflag: 0,
        access: 0,
        dirfd: None,
        extra_type: EXTRA_SOCKET,
        extra: bytes,
        base: 0,
        coflags: 0,
    };
    // SAFETY: the path is a NUL-terminated string on this stack.
    match unsafe { path::connect(&request, path.as_ptr().cast(), &mut []) } {
        Ok(c) => Ok(c.coid),
        Err(ENOENT | ENOSYS) => Err(EAFNOSUPPORT),
        Err(e) => Err(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_socket(domain: c_int, kind: c_int, protocol: c_int) -> c_int {
    match socket(domain, kind, protocol) {
        Ok(fd) => fd,
        Err(EAFNOSUPPORT) => crate::logsock::placeholder(domain, kind).unwrap_or_else(|| fail(EAFNOSUPPORT) as c_int),
        Err(e) => fail(e) as c_int,
    }
}

/// `socketpair()`: the second end, `io-sock` opens as it does a pipe's
/// (`_IO_OPENFD_PIPE` on a new connection to its channel).
///
/// # Safety
///
/// `fds` holds two descriptors.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_socketpair(domain: c_int, kind: c_int, protocol: c_int, fds: *mut c_int) -> c_int {
    let first = match socket(domain, kind, protocol) {
        Ok(fd) => fd,
        Err(e) => return fail(e) as c_int,
    };
    let second = match openfd(first, q::O_RDWR, m::IO_OPENFD_PIPE, kind & SOCK_CLOEXEC != 0) {
        Ok(fd) => fd,
        Err(e) => {
            let _ = crate::fs::__sys_close(first);
            return fail(e) as c_int;
        }
    };
    // SAFETY: the caller's guarantee.
    unsafe {
        fds.write(first);
        fds.add(1).write(second);
    }
    0
}

/// A new connection to the server of `fd`, opened from it with an
/// `_IO_OPENFD` of `xtype`: the descriptor.
fn openfd(fd: c_int, oflag: u32, xtype: u16, cloexec: bool) -> Result<c_int, c_int> {
    let info = server_info(fd)?;
    let new = connect_attach(info.nd, info.pid, info.chid, 0, COF_CLOEXEC)?;
    let msg = Openfd {
        r#type: m::IO_OPENFD,
        combine_len: size_of::<Openfd>() as u16,
        ioflag: ((oflag + 1) & 7) | (oflag & !0x2007) | 0x8000,
        sflag: SH_DENYNO,
        xtype,
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

/// A name's device command on `s`: `DCMD_BIND` or `DCMD_CONNECT`.
///
/// # Safety
///
/// `name` is a `sockaddr` of `len` bytes.
unsafe fn name_cmd(s: c_int, dcmd: u32, name: *const c_void, len: u32) -> c_int {
    if name.is_null() {
        return fail(EFAULT) as c_int;
    }
    let purpose = if dcmd == DCMD_BIND { Use::Bind } else { Use::Connect };
    // SAFETY: the caller's guarantee.
    let fs = match unsafe { fs_name(0, s, name, len, purpose) } {
        Ok(fs) => fs,
        Err(e) => return fail(e) as c_int,
    };
    let (wire, wire_len) = wire_name(&fs, name, len);
    // SAFETY: the name, for reading: a `DEVDIR_TO` command.
    let r = unsafe { devctl(s, (dcmd & 0xffff) | 0x8000_0000 | (wire_len << 16), wire.cast_mut(), wire_len as usize) };
    match r {
        Ok(_) => 0,
        Err(e) => {
            if let (Some(fs), Use::Bind) = (&fs, purpose) {
                // SAFETY: the node's path, a string.
                let _ = unsafe { crate::fs::__sys_unlinkat(-100, fs.path(), 0) };
            }
            fail(if e == ENOSYS { proto::errno::ENOTSOCK } else { e }) as c_int
        }
    }
}

/// What to send for address `name`: its path's name to io-sock, if `fs`
/// holds one, or itself.
pub(super) fn wire_name(fs: &Option<FsName>, name: *const c_void, len: u32) -> (*const c_void, u32) {
    fs.as_ref().map_or((name, len), FsName::name)
}

/// `bind(s, name, len)`.
///
/// # Safety
///
/// `name` is a `sockaddr` of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_bind(s: c_int, name: *const c_void, len: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { name_cmd(s, DCMD_BIND, name, len) }
}

/// `connect(s, name, len)`.
///
/// # Safety
///
/// `name` is a `sockaddr` of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_connect(s: c_int, name: *const c_void, len: u32) -> c_int {
    // SAFETY: the caller's guarantee.
    if let Some(r) = unsafe { crate::logsock::connect(s, name, len) } {
        return r;
    }
    // SAFETY: the caller's guarantee.
    unsafe { name_cmd(s, DCMD_CONNECT, name, len) }
}

/// An int's device command on `s`.
fn int_cmd(s: c_int, dcmd: u32, value: c_int) -> c_int {
    let mut n = value;
    // SAFETY: an int, as the command says.
    match unsafe { devctl(s, dcmd, (&raw mut n).cast(), 4) } {
        Ok(_) => 0,
        Err(ENOSYS) => fail(proto::errno::ENOTSOCK) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_listen(s: c_int, backlog: c_int) -> c_int {
    int_cmd(s, DCMD_LISTEN, backlog)
}

#[unsafe(no_mangle)]
pub extern "C" fn __sys_shutdown(s: c_int, how: c_int) -> c_int {
    int_cmd(s, DCMD_SHUTDOWN, how)
}

/// A name of socket `s`, of the command's kind.
///
/// # Safety
///
/// `name` is writable for `*len` bytes.
unsafe fn name_of(s: c_int, dcmd: u32, name: *mut c_void, len: *mut u32) -> c_int {
    if name.is_null() || len.is_null() {
        return fail(EFAULT) as c_int;
    }
    // SAFETY: the caller's guarantee.
    let cap = unsafe { *len };
    // SAFETY: as above; a `DEVDIR_FROM` command of `cap` bytes.
    match unsafe { devctl(s, (dcmd & 0xffff) | 0x4000_0000 | (cap << 16), name, cap as usize) } {
        Ok(n) => {
            // SAFETY: as above.
            unsafe { *len = (n as u32).min(cap) };
            0
        }
        Err(ENOSYS) => fail(proto::errno::ENOTSOCK) as c_int,
        Err(e) => fail(e) as c_int,
    }
}

/// `getsockname(s, name, len)`.
///
/// # Safety
///
/// `name` is writable for `*len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getsockname(s: c_int, name: *mut c_void, len: *mut u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { name_of(s, DCMD_GETSOCKNAME, name, len) }
}

/// `getpeername(s, name, len)`.
///
/// # Safety
///
/// `name` is writable for `*len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_getpeername(s: c_int, name: *mut c_void, len: *mut u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { name_of(s, DCMD_GETPEERNAME, name, len) }
}

/// Takes a connection: `io-sock` opens it for this process, as a new
/// connection to its channel.
///
/// # Safety
///
/// `addr` is null, or writable for `*addrlen` bytes; `addrlen` is null,
/// or valid.
unsafe fn take(s: c_int, addr: *mut c_void, addrlen: *mut u32, kind: u32, flags: c_int) -> c_int {
    let cap = if addrlen.is_null() {
        0
    } else {
        // SAFETY: the caller's guarantee.
        unsafe { *addrlen }
    };
    if cap != 0 && addr.is_null() {
        return fail(EINVAL) as c_int;
    }
    let info = match server_info(s) {
        Ok(i) => i,
        Err(_) => return fail(EBADF) as c_int,
    };
    let fd = match connect_attach(info.nd, info.pid, info.chid, 0, COF_CLOEXEC) {
        Ok(fd) => fd,
        Err(e) => return fail(e) as c_int,
    };
    let mut msg = Accept {
        r#type: kind,
        flags,
        anamelen: if addr.is_null() { 0 } else { cap },
        ..Accept::default()
    };
    msg.open.r#type = m::IO_OPENFD;
    msg.open.combine_len = 8;
    msg.open.xtype = OPENFD_ACCEPT;
    msg.open.info = client_of(&info, s);
    let n = if msg.anamelen == 0 { 1 } else { 2 };
    let room = msg.anamelen as usize;
    let parts = [Iov::raw((&raw mut msg).cast(), size_of::<Accept>()), Iov::raw(addr, room)];
    // SAFETY: the message, and the caller's address, for its length.
    if let Err(e) = unsafe { request(fd, &parts[..n], &parts[..n]) } {
        let _ = connect_detach(fd);
        return fail(e) as c_int;
    }
    if cap != 0 {
        // SAFETY: the caller's guarantee.
        unsafe { *addrlen = msg.anamelen };
    }
    let bits = (flags as u32 >> 28) & FD_CLOEXEC;
    let _ = kercall::connect_flags(fd, FD_CLOEXEC, bits);
    fd
}

/// `accept(s, addr, addrlen)`.
///
/// # Safety
///
/// `addr` is null, or writable for `*addrlen` bytes; `addrlen` is null,
/// or valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_accept(s: c_int, addr: *mut c_void, addrlen: *mut u32) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { take(s, addr, addrlen, ACCEPT1, 0) }
}

/// `accept4(s, addr, addrlen, flags)`: with `SOCK_CLOEXEC` and
/// `SOCK_NONBLOCK`.
///
/// # Safety
///
/// As `accept()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __sys_accept4(s: c_int, addr: *mut c_void, addrlen: *mut u32, flags: c_int) -> c_int {
    // SAFETY: the caller's guarantee.
    unsafe { take(s, addr, addrlen, ACCEPT4, flags) }
}

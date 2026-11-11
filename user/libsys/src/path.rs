//! Paths: from a pathname to a connection to the server that owns it.
//!
//! QNX has no file system in the kernel. Each server registers the part
//! of the path space it owns with the process manager's path manager,
//! and a client that opens a path asks the path manager first, with an
//! `_IO_CONNECT` message on the process manager's connection. The path
//! manager replies with a link: the servers whose prefixes match the
//! path, longest first. The client connects to each in turn and sends it
//! the same request, with the server's prefix removed from the path,
//! until one accepts; that connection is the result, and for `open()`
//! the file descriptor. A server may answer with a link of its own (a
//! symbolic link, or a path another server owns), which starts the
//! resolution again, or with a file type, when the path names another
//! kind of object. This is `<sys/iomsg.h>`'s connect protocol, as
//! QNX's libc and oscraft-platform's `path` module drive it.

use core::ffi::{c_char, c_int};

use proto::connect::{
    CONNECT_COMBINE_CLOSE, Connect, EFLAG_PATH_NEEDED, EFLAG_SYSTEM_ABS, Entry, FTYPE_ALL,
    LinkReply, RET_CHROOT, RET_FTYPE, RET_LINK, RET_MSG, RET_UMASK,
};
use proto::errno::{EBADMSG, ELOOP, ENAMETOOLONG, ENOENT, ENOEXEC, ENOSYS, ENOTDIR, ENOTSUP, EXDEV};
use proto::fcntl::{O_ACCMODE, O_CLOEXEC, O_DIRFD, O_EXEC, O_NOSYMLINK};
use proto::io::{FdinfoReq, IO_CONNECT, IO_FDINFO};

use crate::kercall::{Iov, SYSMGR_COID, connect_attach, connect_detach, msg_sendv};

/// The longest path, `PATH_MAX`, with its NUL.
pub const PATH_MAX: usize = 1024;
const MAX_LINKS: u32 = 16;
const COF_CLOEXEC: c_int = 1;
const MAX_ENTRIES: usize = 0x30;
const REPLY_MAX: usize = size_of::<LinkReply>() + MAX_ENTRIES * size_of::<Entry>();

const PATH_MANAGER: Entry = Entry {
    nd: 0,
    pid: 1,
    chid: 1,
    handle: 0,
    reserved: 0,
    file_type: FTYPE_ALL,
    prefix_len: 0,
    zero: [0; 3],
};

/// What a connect request asks: the message's fields (QNX's values), the
/// extra data (a combined I/O message), and how to connect.
pub struct Request<'a> {
    pub subtype: u16,
    pub file_type: u32,
    /// QNX's `O_*` flags; the access mode becomes `_IO_FLAG_RD` and `_WR`.
    pub ioflag: u32,
    pub mode: u32,
    pub sflag: u16,
    pub access: u16,
    /// The directory a relative path starts from; `None` for the
    /// working directory.
    pub dirfd: Option<u16>,
    pub extra_type: u8,
    pub extra: &'a [u8],
    /// The lowest connection ID to use: 0 for a file descriptor,
    /// `SYSMGR_COID` for a connection that is not one.
    pub base: u32,
    pub coflags: c_int,
}

/// Which server an entry names: its node, process, channel and handle.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ServerId {
    nd: u32,
    pid: i32,
    chid: i32,
    handle: u32,
}

impl ServerId {
    fn of(entry: &Entry) -> Self {
        ServerId { nd: entry.nd, pid: entry.pid, chid: entry.chid, handle: entry.handle }
    }
}

/// For `rename()` and `link()`, whose two paths must be on one server:
/// the server that owns the first path, and the request to send it with
/// the second. Every other server along the second path's resolution
/// only gets a probe: if one of them accepts the path, it is on another
/// server, EXDEV.
pub struct SameServer<'a> {
    pub server: ServerId,
    pub subtype: u16,
    pub extra_type: u8,
    pub extra: &'a [u8],
}

/// The connection to the server that accepted a request, its status,
/// which server it was and how much of the path it owns.
pub struct Connection {
    pub coid: c_int,
    pub status: c_int,
    pub server: ServerId,
    pub prefix_len: usize,
}

impl Connection {
    pub fn detach(self) {
        let _ = connect_detach(self.coid);
    }
}

#[repr(C, align(8))]
struct Buffers {
    reply: [u8; REPLY_MAX],
    path: [u8; PATH_MAX],
}

struct Servers {
    list: [Entry; MAX_ENTRIES],
    count: usize,
    path_manager: bool,
}

impl Servers {
    fn path_manager() -> Self {
        Servers {
            list: [PATH_MANAGER; MAX_ENTRIES],
            count: 1,
            path_manager: true,
        }
    }
}

/// Copies the NUL-terminated `path` into `buffer`, with its NUL.
///
/// # Safety
///
/// `path` is a string.
unsafe fn copy_path(path: *const c_char, buffer: &mut [u8; PATH_MAX]) -> Result<usize, c_int> {
    for (i, slot) in buffer.iter_mut().enumerate() {
        // SAFETY: the caller's guarantee: up to the NUL.
        let c = unsafe { path.wrapping_add(i).read() } as u8;
        *slot = c;
        if c == 0 {
            return if i == 0 { Err(ENOENT) } else { Ok(i + 1) };
        }
    }
    Err(ENAMETOOLONG)
}

/// `path` (`len` bytes with its NUL) made absolute: directory `fd`'s path,
/// asked of its server, before it. The new length.
fn from_dir(fd: c_int, path: &mut [u8; PATH_MAX], len: usize) -> Result<usize, c_int> {
    let req = FdinfoReq { r#type: IO_FDINFO, combine_len: size_of::<FdinfoReq>() as u16, flags: 0, path_len: PATH_MAX as u32, reserved: 0 };
    let mut head = [0u8; 0x48];
    let mut dir = [0u8; PATH_MAX];
    let send = [Iov::of(&req)];
    let reply = [Iov::raw(head.as_mut_ptr().cast(), head.len()), Iov::raw(dir.as_mut_ptr().cast(), dir.len())];
    // SAFETY: the buffers are on this stack.
    unsafe { msg_sendv(fd, &send, &reply) }.map_err(|e| if e == ENOSYS { ENOTDIR } else { e })?;
    let n = dir.iter().position(|&c| c == 0).unwrap_or(0);
    if n == 0 || dir[0] != b'/' {
        return Err(ENOTDIR);
    }
    let total = n + 1 + len;
    if total > PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    path.copy_within(..len, n + 1);
    path[..n].copy_from_slice(&dir[..n]);
    path[n] = b'/';
    Ok(total)
}

/// Sends the connect message to one server: the header, the path, and
/// the extra data aligned to 8 bytes after the path. The server's
/// status, or its error.
///
/// # Safety
///
/// `coid` is a connection; the extra data is what its type says.
unsafe fn send(
    coid: c_int,
    msg: &Connect,
    path: &[u8],
    extra: &[u8],
    reply: &mut [u8; REPLY_MAX],
) -> Result<i64, c_int> {
    static PAD: [u8; 8] = [0; 8];
    let pad = path.len().div_ceil(8) * 8 - path.len();
    let parts = [
        Iov::of(msg),
        Iov::raw(path.as_ptr().cast(), path.len()),
        Iov::raw(PAD.as_ptr().cast(), pad),
        Iov::raw(extra.as_ptr().cast(), extra.len()),
    ];
    let count = if extra.is_empty() { 2 } else { 4 };
    let back = [Iov::raw(reply.as_mut_ptr().cast(), REPLY_MAX)];
    // SAFETY: every part is valid for its length (the caller's word for
    // the extra data).
    unsafe { msg_sendv(coid, &parts[..count], &back) }
}

/// Resolves `path` and sends the request to the server that owns it;
/// `response` gets the start of the server's reply, for a combined
/// message's answer.
///
/// # Safety
///
/// `path` is a string; the request's extra data is what its type says.
pub unsafe fn connect(
    request: &Request<'_>,
    path: *const c_char,
    response: &mut [u8],
) -> Result<Connection, c_int> {
    // SAFETY: the caller's guarantee.
    unsafe { connect_ex(request, path, response, None, None) }
}

/// [`connect`], with the resolved path, absolute, copied into `resolved`
/// (the path manager rewrites the path it is asked for with
/// `_IO_CONNECT_EFLAG_PATH_NEEDED`).
///
/// # Safety
///
/// As [`connect`].
pub unsafe fn connect_resolved(
    request: &Request<'_>,
    path: *const c_char,
    response: &mut [u8],
    resolved: Option<&mut [u8; PATH_MAX]>,
) -> Result<Connection, c_int> {
    // SAFETY: the caller's guarantee.
    unsafe { connect_ex(request, path, response, resolved, None) }
}

/// [`connect_resolved`], with `same`'s rule for the servers.
///
/// # Safety
///
/// As [`connect`].
pub unsafe fn connect_ex(
    request: &Request<'_>,
    path: *const c_char,
    response: &mut [u8],
    mut resolved: Option<&mut [u8; PATH_MAX]>,
    same: Option<&SameServer<'_>>,
) -> Result<Connection, c_int> {
    let mut buffers = Buffers {
        reply: [0; REPLY_MAX],
        path: [0; PATH_MAX],
    };
    // SAFETY: the caller's guarantee.
    let mut len = unsafe { copy_path(path, &mut buffers.path) }?;
    // A path relative to a directory of another server than the path
    // manager, which knows only its own (procnto's `pathmgr_dir_node`):
    // from the directory's path, as its server says it (`iofdinfo()`).
    let mut dirfd = request.dirfd;
    if let Some(fd) = dirfd
        && buffers.path[0] != b'/'
        && crate::fd::server_info(c_int::from(fd)).is_ok_and(|i| i.pid != 1)
    {
        len = from_dir(c_int::from(fd), &mut buffers.path, len)?;
        dirfd = None;
    }
    let mut msg = Connect {
        r#type: IO_CONNECT,
        subtype: request.subtype,
        file_type: request.file_type,
        reply_max: REPLY_MAX as u16,
        entry_max: MAX_ENTRIES as u16,
        ioflag: (request.ioflag & !(O_ACCMODE | O_CLOEXEC))
            | if request.ioflag & O_EXEC != 0 { 0 } else { (request.ioflag + 1) & O_ACCMODE & u32::from(request.access) },
        mode: request.mode,
        sflag: request.sflag,
        access: request.access,
        extra_type: request.extra_type,
        extra_len: request.extra.len() as u16,
        ..Connect::default()
    };
    if let Some(dirfd) = dirfd {
        msg.dirfd = dirfd;
        msg.ioflag |= O_DIRFD;
    }
    if len > 3 && buffers.path[0] == b'/' && buffers.path[1] == b'/' && buffers.path[2] == b'/' {
        buffers.path.copy_within(2..len, 0);
        len -= 2;
        msg.eflag |= EFLAG_SYSTEM_ABS;
    }
    let coflags = request.coflags | if request.ioflag & O_CLOEXEC != 0 { COF_CLOEXEC } else { 0 };
    let mut links = if request.ioflag & O_NOSYMLINK != 0 { 0 } else { MAX_LINKS };
    let mut servers = Servers::path_manager();
    let mut error = ENOENT;
    let mut said_enoent = false;

    'restart: loop {
        let mut i = 0;
        while i < servers.count {
            let server = servers.list[i];
            i += 1;
            if msg.file_type != 0 && server.file_type != FTYPE_ALL && server.file_type != msg.file_type
            {
                continue;
            }
            let coid = if servers.path_manager {
                if resolved.is_some() && msg.eflag & EFLAG_SYSTEM_ABS == 0 {
                    msg.eflag |= EFLAG_PATH_NEEDED;
                } else {
                    msg.eflag &= !EFLAG_PATH_NEEDED;
                }
                SYSMGR_COID
            } else {
                let coid = connect_attach(server.nd, server.pid, server.chid, request.base, coflags)?;
                msg.eflag &= !EFLAG_SYSTEM_ABS;
                coid
            };
            let prefix = usize::from(server.prefix_len).min(len - 1);
            msg.path_len = (len - prefix) as u16;
            msg.handle = server.handle;
            let id = ServerId::of(&server);
            let mut extra = request.extra;
            if let Some(same) = same {
                if id == same.server {
                    msg.subtype = same.subtype;
                    msg.extra_type = same.extra_type;
                    extra = same.extra;
                } else {
                    msg.subtype = CONNECT_COMBINE_CLOSE;
                    msg.extra_type = 0;
                    extra = &[];
                }
                msg.extra_len = extra.len() as u16;
            }
            // A QNX binary reads QNX's flags.
            let wire = if servers.path_manager || crate::fd::native(coid) {
                msg
            } else {
                Connect { ioflag: qnxabi::fcntl::ioflag_to_qnx(msg.ioflag), ..msg }
            };
            // SAFETY: the caller's guarantee for the extra data.
            let sent = unsafe { send(coid, &wire, &buffers.path[prefix..len], extra, &mut buffers.reply) };
            let status = match sent {
                Ok(status) => status as c_int,
                Err(e) => {
                    if coid != SYSMGR_COID {
                        let _ = connect_detach(coid);
                    }
                    if !servers.path_manager && let Some(same) = same {
                        // The server that owns both paths refused: that is
                        // the answer. Any other only did not have the path.
                        if id == same.server {
                            return Err(e);
                        }
                        error = e;
                        continue;
                    }
                    // As QNX's library: a server's ENOENT wins over
                    // another's ENOSYS; any other error is the answer.
                    match e {
                        ENOENT => {
                            error = ENOENT;
                            said_enoent = true;
                            continue;
                        }
                        ENOSYS => {
                            if !said_enoent {
                                error = ENOSYS;
                            }
                            continue;
                        }
                        other => return Err(other),
                    }
                }
            };
            if status >= 0 {
                if same.is_some_and(|s| s.server != id) {
                    let _ = connect_detach(coid);
                    return Err(EXDEV);
                }
                let n = response.len().min(REPLY_MAX);
                response[..n].copy_from_slice(&buffers.reply[..n]);
                if let Some(out) = resolved.as_deref_mut() {
                    // The root for the empty path; nothing if it does not fit.
                    let p: &[u8] = if len <= 1 { b"/\0" } else { &buffers.path[..len] };
                    if p.len() <= PATH_MAX {
                        out[..p.len()].copy_from_slice(p);
                    } else {
                        out[0] = 0;
                    }
                }
                return Ok(Connection { coid, status, server: id, prefix_len: prefix });
            }
            if coid != SYSMGR_COID {
                let _ = connect_detach(coid);
            }
            let flags = status as u32;
            if flags & RET_LINK != 0 {
                // SAFETY: the reply buffer holds a link reply.
                let link = unsafe { buffers.reply.as_ptr().cast::<LinkReply>().read() };
                let n = usize::from(link.nentries);
                if n > MAX_ENTRIES {
                    return Err(EBADMSG);
                }
                let at = size_of::<LinkReply>() + n * size_of::<Entry>();
                len = follow(&mut buffers, at, usize::from(link.path_len), prefix, len)?;
                msg.eflag |= EFLAG_SYSTEM_ABS;
                if n == 0 {
                    // A symbolic link: resolve its target from the start.
                    if links == 0 {
                        return Err(ELOOP);
                    }
                    links -= 1;
                    servers = Servers::path_manager();
                    continue 'restart;
                }
                if servers.path_manager {
                    let _ = flags & RET_CHROOT; // a root of its own: not kept yet
                    msg.eflag = link.eflag;
                    if flags & RET_UMASK != 0 {
                        msg.mode &= !(link.umask & !0xf000);
                    }
                }
                for (k, slot) in servers.list.iter_mut().take(n).enumerate() {
                    let offset = size_of::<LinkReply>() + k * size_of::<Entry>();
                    // SAFETY: the reply holds n entries after its header.
                    *slot = unsafe { buffers.reply[offset..].as_ptr().cast::<Entry>().read_unaligned() };
                }
                servers.count = n;
                servers.path_manager = false;
                continue 'restart;
            }
            if flags & RET_FTYPE != 0 {
                let r = &buffers.reply;
                let file_type = u32::from_ne_bytes([r[4], r[5], r[6], r[7]]);
                if file_type == FTYPE_ALL {
                    return Err(c_int::from(u16::from_ne_bytes([r[0], r[1]])));
                }
                msg.file_type = file_type;
                msg.eflag |= EFLAG_SYSTEM_ABS;
                servers = Servers::path_manager();
                continue 'restart;
            }
            if flags & RET_MSG != 0 {
                // A hand-over to a named channel's server: not yet.
                return Err(ENOTSUP);
            }
            return Err(ENOEXEC);
        }
        // With two paths, a second that no server would take with the
        // first is on another server.
        if same.is_some() && (error == ENOENT || error == ENOSYS) {
            return Err(EXDEV);
        }
        return Err(error);
    }
}

/// Replaces the path being resolved with a link reply's, at `at` in the
/// reply. A relative path is relative to the prefix of the server that
/// replied.
fn follow(buffers: &mut Buffers, at: usize, path_len: usize, prefix: usize, len: usize) -> Result<usize, c_int> {
    let rest = &buffers.reply[at.min(REPLY_MAX)..];
    let new_len = match path_len {
        0 => rest.iter().position(|&c| c == 0).ok_or(EBADMSG)? + 1,
        n if n <= rest.len() && rest[n - 1] == 0 => n,
        _ => return Err(EBADMSG),
    };
    let keep = if rest[0] == b'/' || rest[0] == 0 { 0 } else { prefix.min(len) };
    if keep + new_len > PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let (reply, path) = (&buffers.reply, &mut buffers.path);
    path[keep..keep + new_len].copy_from_slice(&reply[at..at + new_len]);
    if new_len == 1 && keep == 0 {
        path[0] = 0;
    }
    Ok(keep + new_len)
}

/// A probe of `path` with a combined message, closed at once: the reply
/// and the server's status.
///
/// # Safety
///
/// `path` is a string.
pub unsafe fn probe(
    dirfd: Option<u16>,
    mode: u32,
    extra: &[u8],
    path: *const c_char,
    response: &mut [u8],
) -> Result<c_int, c_int> {
    use proto::fcntl::{O_LARGEFILE, O_NOCTTY, O_NONBLOCK, SH_DENYNO};
    let request = Request {
        subtype: CONNECT_COMBINE_CLOSE,
        file_type: 0,
        ioflag: O_LARGEFILE | O_NOCTTY | O_NONBLOCK,
        mode,
        sflag: SH_DENYNO,
        access: 0,
        dirfd,
        extra_type: 0,
        extra,
        base: SYSMGR_COID as u32,
        coflags: 0,
    };
    // SAFETY: the caller's guarantee.
    let c = unsafe { connect(&request, path, response) }?;
    let status = c.status;
    c.detach();
    Ok(status)
}

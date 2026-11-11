//! Paths: from a pathname to a connection to the server that owns it.
//!
//! QNX has no file system in the kernel. Each server, a *resource
//! manager*, registers the part of the path space it owns with the
//! process manager's *path manager*, and a client that opens a path asks
//! the path manager first: an `_IO_CONNECT` message, on the process
//! manager's connection. The path manager does not open anything. It
//! replies with a *link*: the servers whose prefixes match the path,
//! longest first, each with the length of the prefix it owns. The client
//! then connects to each server in turn and sends it the same
//! `_IO_CONNECT`, with the server's prefix removed from the path, until
//! one accepts. The connection to the server that accepted is the result:
//! for `open()`, the file descriptor.
//!
//! A server may answer with a link of its own (a symbolic link, or a path
//! another server owns), which starts the resolution again, at most 16
//! times; or with a file type, when the path names a different kind of
//! object. `ENOENT` from one server only means the next may have the
//! path. This module is that protocol, `<sys/iomsg.h>`'s connect
//! messages; the functions that use it (`open()`, `stat()`, `unlink()`
//! and the rest) differ only in the message's fields.

use core::ffi::{c_char, c_int, c_long, c_uint, c_void};

use crate::abi::{Iov, SYSMGR_COID};
use crate::neutrino::connect::{ConnectAttach_r, ConnectDetach_r};
use crate::neutrino::msg::{MsgSendv, MsgSendvnc};
use crate::tls;

pub use proto::connect::{
    CONNECT_COMBINE, CONNECT_COMBINE_CLOSE, CONNECT_LINK, CONNECT_MKNOD, CONNECT_OPEN,
    CONNECT_READLINK, CONNECT_RENAME, CONNECT_UNLINK,
};
use proto::connect::{
    Connect as IoConnect, EFLAG_PATH_NEEDED, EFLAG_SYSTEM_ABS, Entry, FTYPE_ALL, LinkReply,
    RET_CHROOT, RET_FTYPE, RET_LINK, RET_MSG, RET_UMASK,
};
use proto::errno::{EBADMSG, ELOOP, ENAMETOOLONG, ENOENT, ENOEXEC, ENOMEM, ENOSYS};
use proto::errno::{ENOTSUP, EXDEV};
use proto::qnx_fcntl::{O_ACCMODE, O_CLOEXEC, O_DIRFD, O_NOSYMLINK};
use proto::io::IO_CONNECT;

/// The longest path, `PATH_MAX`, with its NUL.
pub const PATH_MAX: usize = 1024;

/// How many links a resolution follows before `ELOOP`.
const MAX_LINKS: u32 = 16;

const COF_CLOEXEC: c_int = 1;

/// The most servers a link reply holds, and the reply buffer's size, as
/// the client tells each server (`reply_max`, `entry_max`).
const MAX_ENTRIES: usize = 0x30;
const REPLY_MAX: usize = size_of::<LinkReply>() + MAX_ENTRIES * size_of::<Entry>();

/// The path manager, as the first "server": on the process manager's
/// connection, owning every path from the root.
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

/// What a connect request asks: the message's fields, the extra data
/// (a combined I/O message, a rename's target), and how to connect.
pub struct Request<'a> {
    pub subtype: u16,
    /// `_FTYPE_*`: the kind of server wanted; 0 for any.
    pub file_type: u32,
    /// `O_*` flags; the access mode becomes `_IO_FLAG_RD` and `_WR`.
    pub ioflag: u32,
    /// `S_IF*` and permissions: the type to create or the link to follow.
    pub mode: u32,
    /// `SH_*`: sharing.
    pub sflag: u16,
    /// Which access the caller checks: `_IO_FLAG_*` bits.
    pub access: u16,
    /// The directory a relative path starts from, for the `*at()`
    /// functions; `None` for the working directory.
    pub dirfd: Option<u16>,
    pub extra_type: u8,
    pub extra: &'a [u8],
    /// The lowest connection ID to use: 0 for a file descriptor, or
    /// `SYSMGR_COID`'s range for a connection that is not one.
    pub base: c_uint,
    /// Whether the messages are cancellation points (`MsgSendv()`), as
    /// `open()`'s are, or not (`MsgSendvnc()`).
    pub cancellable: bool,
    /// `_NTO_COF_*` flags for the connection, beyond `O_CLOEXEC`'s:
    /// `_NTO_COF_NOSHARE`, say, for a connection of its own.
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
        ServerId {
            nd: entry.nd,
            pid: entry.pid,
            chid: entry.chid,
            handle: entry.handle,
        }
    }
}

/// The result of a request: the connection to the server that accepted
/// it, the server's status, which server it was, and how much of the path
/// it owns.
pub struct Connection {
    pub coid: c_int,
    pub status: c_int,
    pub server: ServerId,
    pub prefix_len: usize,
    /// How much of the resolved path is the process's root directory, if
    /// it has one of its own (`chroot()`): 0 if not.
    pub chroot_len: usize,
}

/// For `rename()` and `link()`, whose two paths must be on one server:
/// the server that owns the first path, and the request to send it with
/// the second. Every other server along the second path's resolution only
/// gets a probe (`_IO_CONNECT_COMBINE_CLOSE`, nothing extra): if one of
/// them accepts the path, it is on another server, `EXDEV`.
pub struct SameServer<'a> {
    pub server: ServerId,
    pub subtype: u16,
    pub extra_type: u8,
    pub extra: &'a [u8],
}

impl Connection {
    /// Detaches the connection, when the request was all that was wanted.
    pub fn detach(self) {
        ConnectDetach_r(self.coid);
    }
}

/// The reply buffer: the link reply's room, then room for the path being
/// resolved, which a link replaces.
#[repr(C, align(8))]
struct Buffers {
    reply: [u8; REPLY_MAX],
    path: [u8; PATH_MAX],
}

/// The servers to try, copied out of a link reply before the next reply
/// overwrites it.
struct Servers {
    list: [Entry; MAX_ENTRIES],
    count: usize,
    /// Whether the list is the path manager alone.
    path_manager: bool,
}

impl Servers {
    fn path_manager() -> Self {
        let mut list = [PATH_MANAGER; MAX_ENTRIES];
        list[0] = PATH_MANAGER;
        Servers {
            list,
            count: 1,
            path_manager: true,
        }
    }
}

/// Copies the NUL-terminated `path` into `buffer`, with its NUL; returns
/// its length with the NUL.
///
/// # Safety
///
/// `path` is a string.
unsafe fn copy_path(path: *const c_char, buffer: &mut [u8; PATH_MAX]) -> Result<usize, c_int> {
    for (i, slot) in buffer.iter_mut().enumerate() {
        // SAFETY: the caller's guarantee: up to the NUL.
        let c = unsafe { path.wrapping_add(i).read() }.to_ne_bytes()[0];
        *slot = c;
        if c == 0 {
            return if i == 0 { Err(ENOENT) } else { Ok(i + 1) };
        }
    }
    Err(ENAMETOOLONG)
}

/// Resolves `path` and sends the request to the server that owns it;
/// `response` gets the start of the server's reply, for a combined
/// message's answer. On failure, the error number.
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
    unsafe { connect_ex(request, path, response, None, None, None) }
}

/// [`connect`], with the resolved path, absolute, copied into `resolved`
/// (empty if it does not fit), and with `same`'s rule for the servers.
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
    mut each: Option<&mut dyn FnMut(c_int) -> bool>,
) -> Result<Connection, c_int> {
    let mut buffers = Buffers {
        reply: [0; REPLY_MAX],
        path: [0; PATH_MAX],
    };
    // SAFETY: the caller's guarantee.
    let mut len = unsafe { copy_path(path, &mut buffers.path) }?;
    let mut msg = IoConnect {
        r#type: IO_CONNECT,
        subtype: request.subtype,
        file_type: request.file_type,
        reply_max: REPLY_MAX as u16,
        entry_max: MAX_ENTRIES as u16,
        // The access mode, O_RDONLY 0 to O_RDWR 2, becomes _IO_FLAG_RD 1
        // to _IO_FLAG_RD | _IO_FLAG_WR 3, of what the caller checks.
        ioflag: (request.ioflag & !(O_ACCMODE | O_CLOEXEC))
            | ((request.ioflag + 1) & O_ACCMODE & u32::from(request.access)),
        mode: request.mode,
        sflag: request.sflag,
        access: request.access,
        extra_type: request.extra_type,
        extra_len: request.extra.len() as u16,
        ..IoConnect::default()
    };
    if let Some(dirfd) = request.dirfd {
        msg.dirfd = dirfd;
        msg.ioflag |= O_DIRFD;
    }
    // "//" at the start is a network path; "///" the system's root.
    if len > 3 && buffers.path.starts_with(b"///") {
        buffers.path.copy_within(2..len, 0);
        len -= 2;
        msg.eflag |= EFLAG_SYSTEM_ABS;
    }
    let coflags = request.coflags
        | if request.ioflag & O_CLOEXEC != 0 {
            COF_CLOEXEC
        } else {
            0
        };
    let mut links = if request.ioflag & O_NOSYMLINK != 0 {
        0
    } else {
        MAX_LINKS
    };
    let mut servers = Servers::path_manager();
    // The error to report if no server has the path: ENOENT unless one
    // said otherwise.
    let mut error = ENOENT;
    let mut said_enoent = false;
    let saved_errno = tls::get_errno();
    // With a root of its own, the path manager's resolved path is kept
    // when it replies, with the root's length.
    let mut chroot_len = 0;
    // Whether `each` has had a connection.
    let mut collected = false;

    'restart: loop {
        let mut i = 0;
        while i < servers.count {
            let server = servers.list[i];
            i += 1;
            if msg.file_type != 0
                && server.file_type != FTYPE_ALL
                && server.file_type != msg.file_type
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
                let coid =
                    ConnectAttach_r(server.nd, server.pid, server.chid, request.base, coflags);
                if coid < 0 {
                    return Err(-coid);
                }
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
            // SAFETY: the parts are valid for their lengths.
            let status = unsafe {
                send(
                    request,
                    coid,
                    &msg,
                    &buffers.path[prefix..len],
                    extra,
                    &mut buffers.reply,
                )
            };
            if status >= 0 {
                if same.is_some_and(|s| s.server != id) {
                    ConnectDetach_r(coid);
                    return Err(EXDEV);
                }
                tls::set_errno(saved_errno);
                let n = response.len().min(REPLY_MAX);
                response[..n].copy_from_slice(&buffers.reply[..n]);
                if chroot_len == 0
                    && let Some(out) = resolved.as_deref_mut()
                {
                    copy_resolved(out, &buffers.path[..len]);
                }
                // A union mount's directory: every server's connection is
                // wanted, not only the first.
                if let Some(each) = each.as_deref_mut() {
                    if !each(coid) {
                        return Err(ENOMEM);
                    }
                    collected = true;
                    continue;
                }
                return Ok(Connection {
                    coid,
                    status,
                    server: id,
                    prefix_len: prefix,
                    chroot_len,
                });
            }
            if coid != SYSMGR_COID {
                ConnectDetach_r(coid);
            }
            if status == -1
                && !servers.path_manager
                && let Some(same) = same
            {
                // The server that owns both paths refused: that is the
                // answer. Any other only did not have the path.
                if id == same.server {
                    return Err(tls::get_errno());
                }
                error = tls::get_errno();
                continue;
            }
            if status == -1 {
                // This server failed; the next may have the path.
                // As QNX's library does: a server's ENOENT wins over
                // another's ENOSYS; ENOSYS only if no server said ENOENT.
                match tls::get_errno() {
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
                    if flags & RET_CHROOT != 0 && link.chroot_len != 0 {
                        chroot_len = usize::from(link.chroot_len);
                        if let Some(out) = resolved.as_deref_mut() {
                            copy_resolved(out, &buffers.path[..len]);
                        }
                    }
                    msg.eflag = link.eflag;
                    if flags & RET_UMASK != 0 {
                        msg.mode &= !(link.umask & !0xf000);
                    }
                }
                for (k, slot) in servers.list.iter_mut().take(n).enumerate() {
                    let offset = size_of::<LinkReply>() + k * size_of::<Entry>();
                    let at = buffers.reply[offset..].as_ptr().cast::<Entry>();
                    // SAFETY: the reply holds n entries after its header,
                    // within the buffer (n is at most MAX_ENTRIES).
                    *slot = unsafe { at.read_unaligned() };
                }
                servers.count = n;
                servers.path_manager = false;
                continue 'restart;
            }
            if flags & RET_FTYPE != 0 {
                // The path names another type of object: ask again for it.
                // The reply is a file type reply: an error, then the type.
                let file_type = u32::from_ne_bytes([
                    buffers.reply[4],
                    buffers.reply[5],
                    buffers.reply[6],
                    buffers.reply[7],
                ]);
                if file_type == FTYPE_ALL {
                    let error = u16::from_ne_bytes([buffers.reply[0], buffers.reply[1]]);
                    return Err(c_int::from(error));
                }
                msg.file_type = file_type;
                msg.eflag |= EFLAG_SYSTEM_ABS;
                servers = Servers::path_manager();
                continue 'restart;
            }
            if flags & RET_MSG != 0 {
                // The process manager hands the client to a server, with a
                // message to send it: a named channel's open, say. Only
                // it (process 1) may.
                if server.pid != 1 {
                    return Err(ENOTSUP);
                }
                // SAFETY: the reply buffer holds the reply.
                return unsafe { hand_over(request, coflags, &mut buffers.reply) };
            }
            return Err(ENOEXEC);
        }
        tls::set_errno(saved_errno);
        if collected {
            return Ok(Connection {
                coid: -1,
                status: 0,
                server: ServerId {
                    nd: 0,
                    pid: 0,
                    chid: 0,
                    handle: 0,
                },
                prefix_len: 0,
                chroot_len,
            });
        }
        // With two paths, a second that no server would take with the
        // first is on another server.
        if same.is_some() && (error == ENOENT || error == ENOSYS) {
            return Err(EXDEV);
        }
        return Err(error);
    }
}

/// Copies the resolved path `path`, with its NUL, into `out`: the root
/// for the empty path; nothing (an empty string) if it does not fit.
fn copy_resolved(out: &mut [u8; PATH_MAX], path: &[u8]) {
    let path: &[u8] = if path.len() <= 1 { b"/\0" } else { path };
    if path.len() <= PATH_MAX {
        out[..path.len()].copy_from_slice(path);
    } else {
        out[0] = 0;
    }
}

/// Replaces the path being resolved with a link reply's, at `at` in the
/// reply, of `path_len` bytes with its NUL (0: up to its NUL). A relative
/// path is relative to the prefix of the server that replied, the first
/// `prefix` bytes of the old path. Returns the new path's length.
fn follow(
    buffers: &mut Buffers,
    at: usize,
    path_len: usize,
    prefix: usize,
    len: usize,
) -> Result<usize, c_int> {
    let rest = &buffers.reply[at.min(REPLY_MAX)..];
    let new_len = match path_len {
        0 => rest.iter().position(|&c| c == 0).ok_or(EBADMSG)? + 1,
        n if n <= rest.len() && rest[n - 1] == 0 => n,
        _ => return Err(EBADMSG),
    };
    let keep = if rest[0] == b'/' || rest[0] == 0 {
        0
    } else {
        prefix.min(len)
    };
    if keep + new_len > PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let (reply, path) = (&buffers.reply, &mut buffers.path);
    path[keep..keep + new_len].copy_from_slice(&reply[at..at + new_len]);
    if new_len == 1 && keep == 0 {
        // An empty path: the server's own root.
        path[0] = 0;
    }
    Ok(keep + new_len)
}

/// Sends the connect message to one server: the header, the path, and
/// the extra data aligned to 8 bytes after the path. Returns the server's
/// status, with `errno` set if it is -1: a connect reply's flags make
/// other statuses negative.
///
/// # Safety
///
/// `coid` is a connection; the request's extra data is valid.
unsafe fn send(
    request: &Request<'_>,
    coid: c_int,
    msg: &IoConnect,
    path: &[u8],
    extra: &[u8],
    reply: &mut [u8; REPLY_MAX],
) -> c_int {
    static PAD: [u8; 8] = [0; 8];
    let pad = path.len().div_ceil(8) * 8 - path.len();
    let parts = [
        Iov {
            base: (msg as *const IoConnect).cast(),
            len: size_of::<IoConnect>(),
        },
        Iov {
            base: path.as_ptr().cast(),
            len: path.len(),
        },
        Iov {
            base: PAD.as_ptr().cast(),
            len: pad,
        },
        Iov {
            base: extra.as_ptr().cast(),
            len: extra.len(),
        },
    ];
    let count = if extra.is_empty() { 2 } else { 4 };
    let reply_part = Iov {
        base: reply.as_mut_ptr().cast::<c_void>(),
        len: REPLY_MAX,
    };
    let send = if request.cancellable {
        MsgSendv
    } else {
        MsgSendvnc
    };
    let status: c_long = send(
        coid,
        parts.as_ptr().cast(),
        count,
        (&raw const reply_part).cast(),
        1,
    );
    status as c_int
}


/// `_IO_CONNECT_RET_MSG`'s reply, `msg_reply_t`: a link reply's header,
/// whose `path_len` is the new message's length; the server, as a
/// `struct _msg_info` of 32-bit fields (node at 0x14, process at 0x1c,
/// channel at 0x24); then the message, at 0x44.
const HANDOVER_INFO: usize = 0x14;
const HANDOVER_MSG: usize = 0x44;

/// Connects to the server the path manager's reply in `reply` names, and
/// sends it the reply's message, if any.
///
/// # Safety
///
/// `reply` holds such a reply.
unsafe fn hand_over(
    request: &Request<'_>,
    coflags: c_int,
    reply: &mut [u8; REPLY_MAX],
) -> Result<Connection, c_int> {
    let word = |r: &[u8; REPLY_MAX], at: usize| {
        u32::from_ne_bytes([r[at], r[at + 1], r[at + 2], r[at + 3]])
    };
    let (nd, pid, chid) = (
        word(reply, HANDOVER_INFO),
        word(reply, HANDOVER_INFO + 8) as i32,
        word(reply, HANDOVER_INFO + 16) as c_int,
    );
    let len =
        usize::from(u16::from_ne_bytes([reply[0x12], reply[0x13]])).min(REPLY_MAX - HANDOVER_MSG);
    let coid = ConnectAttach_r(nd, pid, chid, request.base, coflags);
    if coid < 0 {
        return Err(-coid);
    }
    let mut status = 0;
    if len != 0 {
        let mut message = [0u8; REPLY_MAX - HANDOVER_MSG];
        message[..len].copy_from_slice(&reply[HANDOVER_MSG..HANDOVER_MSG + len]);
        let send = [Iov {
            base: message.as_ptr().cast(),
            len,
        }];
        let back = [Iov {
            base: reply.as_mut_ptr().cast(),
            len: REPLY_MAX,
        }];
        // SAFETY: the parts are valid for their lengths.
        match unsafe { crate::neutrino::msg_sendv(coid, &send, &back) } {
            Ok(s) => status = s as c_int,
            Err(e) => {
                ConnectDetach_r(coid);
                return Err(e);
            }
        }
    }
    Ok(Connection {
        coid,
        status,
        server: ServerId {
            nd,
            pid,
            chid,
            handle: 0,
        },
        prefix_len: 0,
        chroot_len: 0,
    })
}

const _: () = {
    assert!(size_of::<IoConnect>() == 0x28);
    assert!(size_of::<Entry>() == 0x20);
    assert!(size_of::<LinkReply>() == 0x14);
    assert!(REPLY_MAX == 0x614);
};

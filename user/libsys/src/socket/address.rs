//! The path of an `AF_UNIX` address. io-sock does not look paths up: a
//! socket's name is a node in a file system, which the client opens, or
//! makes for `bind()`, and hands to io-sock as a descriptor handle
//! (`dupfd`).

use core::ffi::{c_char, c_int, c_void};

use proto::connect::CONNECT_MKNOD;
use proto::errno::{EADDRINUSE, EBADF, EEXIST, EINVAL};
use proto::fcntl::{self as q, SH_DENYNO};
use proto::stat::S_IFSOCK;

use crate::dupfd;
use crate::errno::{last, set_last};
use crate::fd::server_info;
use crate::path::{self, Request};

const AF_UNIX: u8 = 1;
/// `sizeof(struct sockaddr_un)`.
const SUN_MAX: u32 = 106;
/// FreeBSD's `O_WRONLY | O_CLOEXEC`, to open the node.
const OPEN_NODE: c_int = 0x10_0001;
/// `LOCK_SH`.
const LOCK_SH: c_int = 1;
/// What a descriptor's name is to io-sock: a `sun_path` of a 1, the
/// handle, and for `connect()` eight zeros, its cache's id.
const NAME_BIND: u8 = 11;
const NAME_CONNECT: u8 = 19;

/// What the name is for.
#[derive(Clone, Copy, PartialEq)]
pub enum Use {
    Bind,
    Connect,
}

/// The name a path is to io-sock, holding the node open.
#[repr(C, align(8))]
pub struct FsName {
    name: [u8; NAME_CONNECT as usize],
    node: c_int,
    path: [u8; 105],
}

impl FsName {
    /// The name to send, and its length.
    pub fn name(&self) -> (*const c_void, u32) {
        (self.name.as_ptr().cast(), u32::from(self.name[0]))
    }

    /// The node's path.
    pub fn path(&self) -> *const c_char {
        self.path.as_ptr().cast()
    }
}

impl Drop for FsName {
    fn drop(&mut self) {
        let e = last();
        let _ = crate::fs::__sys_close(self.node);
        set_last(e);
    }
}

/// The name `name` is to io-sock: none if it is no path; for the server
/// of `fd` (`pid` if known, not 0). `Err` with QNX's error.
///
/// # Safety
///
/// `name` is a `sockaddr` of `len` bytes.
pub unsafe fn fs_name(pid: i32, fd: c_int, name: *const c_void, len: u32, purpose: Use) -> Result<Option<FsName>, i32> {
    let bytes = name.cast::<u8>();
    // SAFETY: a sockaddr has its family at 1, the caller's guarantee.
    if len < 3 || unsafe { bytes.add(1).read() } != AF_UNIX || unsafe { bytes.add(2).read() } == 0 {
        return Ok(None);
    }
    if len > SUN_MAX {
        return Err(EINVAL);
    }
    let mut server = pid;
    if server == 0 {
        server = server_info(fd).map_err(|_| EBADF)?.pid;
    }
    let mut path = [0u8; 105];
    let n = len as usize - 2;
    // SAFETY: the n bytes of the path; 104 at most, so the NUL stays.
    unsafe { path.as_mut_ptr().copy_from(bytes.add(2), n) };
    let node = open_node(path.as_ptr().cast(), purpose).map_err(|e| if e == EEXIST { EADDRINUSE } else { e })?;
    let handle = match dupfd::register(server, node, 0) {
        Ok(h) => h,
        Err(e) => {
            let _ = crate::fs::__sys_close(node);
            return Err(e);
        }
    };
    let mut fs = FsName {
        name: [0; NAME_CONNECT as usize],
        node,
        path,
    };
    fs.name[0] = NAME_CONNECT;
    fs.name[1] = AF_UNIX;
    fs.name[2] = 1;
    fs.name[3..11].copy_from_slice(&handle.to_ne_bytes());
    if purpose == Use::Bind {
        fs.name[0] = NAME_BIND;
    } else {
        let _ = crate::misc::__sys_flock(node, LOCK_SH);
    }
    Ok(Some(fs))
}

/// Opens the node, or makes it (`S_IFSOCK`): QNX's error.
fn open_node(path: *const c_char, purpose: Use) -> Result<c_int, i32> {
    match purpose {
        Use::Connect => {
            // SAFETY: a string.
            let fd = unsafe { crate::fs::__sys_open(path, OPEN_NODE, 0) };
            if fd < 0 { Err(qnx_last()) } else { Ok(fd) }
        }
        Use::Bind => {
            let request = Request {
                subtype: CONNECT_MKNOD,
                file_type: 0,
                ioflag: q::O_WRONLY | q::O_CREAT | q::O_EXCL | q::O_CLOEXEC,
                mode: S_IFSOCK | 0o666,
                sflag: SH_DENYNO,
                access: 2,
                dirfd: None,
                extra_type: 0,
                extra: &[],
                base: 0,
                coflags: 0,
            };
            // SAFETY: a string, and no extra data.
            unsafe { path::connect(&request, path, &mut []) }.map(|c| c.coid)
        }
    }
}

/// The last error as QNX's number.
fn qnx_last() -> i32 {
    let e = last();
    qnxabi::errno::to_qnx(e).unwrap_or(e)
}

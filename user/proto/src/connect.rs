//! `_IO_CONNECT` (`<sys/iomsg.h>`): a request by path, to the path
//! manager and then to each server it names: open, unlink, rename,
//! make a node, read or make a link, mount. A server may answer with
//! the servers to try next (a link reply) or with a message for the
//! client to send to another server.

use crate::pod::pod;

/// `struct _io_connect` up to its path, which follows it with its NUL
/// and the extra data (C's `sizeof` counts the path's first byte).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Connect {
    pub r#type: u16,
    pub subtype: u16,
    pub file_type: u32,
    pub reply_max: u16,
    pub entry_max: u16,
    pub reserved: u32,
    pub handle: u32,
    pub ioflag: u32,
    pub mode: u32,
    pub sflag: u16,
    pub access: u16,
    pub dirfd: u16,
    pub path_len: u16,
    pub eflag: u8,
    pub extra_type: u8,
    pub extra_len: u16,
}

/// `_IO_CONNECT_*`: the request.
pub const CONNECT_COMBINE: u16 = 0;
pub const CONNECT_COMBINE_CLOSE: u16 = 1;
pub const CONNECT_OPEN: u16 = 2;
pub const CONNECT_UNLINK: u16 = 3;
pub const CONNECT_RENAME: u16 = 4;
pub const CONNECT_MKNOD: u16 = 5;
pub const CONNECT_READLINK: u16 = 6;
pub const CONNECT_LINK: u16 = 7;
pub const CONNECT_RSVD_UNBLOCK: u16 = 8;
pub const CONNECT_MOUNT: u16 = 9;

/// `_IO_CONNECT_EXTRA_*`: what the extra data is.
pub const EXTRA_NONE: u8 = 0;
pub const EXTRA_SYMLINK: u8 = 2;
pub const EXTRA_MQUEUE: u8 = 3;
pub const EXTRA_SOCKET: u8 = 5;
pub const EXTRA_SEM: u8 = 6;
pub const EXTRA_RESMGR_LINK: u8 = 7;
pub const EXTRA_PROC_SYMLINK: u8 = 8;
pub const EXTRA_RENAME: u8 = 9;
pub const EXTRA_MOUNT: u8 = 10;
pub const EXTRA_MOUNT_OCB: u8 = 11;
pub const EXTRA_TYMEM: u8 = 12;
pub const EXTRA_LINK: u8 = 13;

/// `_IO_CONNECT_EFLAG_*`.
pub const EFLAG_DIR: u8 = 0x01;
pub const EFLAG_DOT: u8 = 0x02;
pub const EFLAG_DOTDOT: u8 = 0x04;
pub const EFLAG_SYSTEM_ABS: u8 = 0x08;
pub const EFLAG_PATH_NEEDED: u8 = 0x10;

/// `_IO_CONNECT_RET_*`: what a server's status says it replied.
pub const RET_FTYPE: u32 = 0x0000_8000;
pub const RET_LINK: u32 = 0x0001_0000;
pub const RET_UMASK: u32 = 0x0002_0000;
pub const RET_NOCTTY: u32 = 0x0004_0000;
pub const RET_CHROOT: u32 = 0x0008_0000;
pub const RET_MSG: u32 = 0x0010_0000;
pub const RET_TYPE_MASK: u32 = 0x0001_e000;
pub const RET_FLAG: u32 = 0x8000_0000;

/// `struct _io_connect_link_reply`, followed by `nentries` entries and
/// the path.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LinkReply {
    pub reserved1: u32,
    pub file_type: u32,
    pub eflag: u8,
    pub reserved2: u8,
    pub chroot_len: u16,
    pub umask: u32,
    pub nentries: u16,
    pub path_len: u16,
}

/// `struct _io_connect_entry`: one server of a link reply.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Entry {
    pub nd: u32,
    pub pid: i32,
    pub chid: i32,
    pub handle: u32,
    pub reserved: u32,
    pub file_type: u32,
    pub prefix_len: u16,
    pub zero: [u16; 3],
}

/// `struct _io_connect_ftype_reply`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FtypeReply {
    pub status: u16,
    pub reserved: u16,
    pub file_type: u32,
}

/// `_FTYPE_*` (`<sys/ftype.h>`): what kind of file a server serves.
pub const FTYPE_MATCHED: u32 = u32::MAX;
pub const FTYPE_ALL: u32 = u32::MAX;
pub const FTYPE_ANY: u32 = 0;
pub const FTYPE_FILE: u32 = 1;
pub const FTYPE_LINK: u32 = 2;
pub const FTYPE_SYMLINK: u32 = 3;
pub const FTYPE_PIPE: u32 = 4;
pub const FTYPE_SHMEM: u32 = 5;
pub const FTYPE_MQUEUE: u32 = 6;
pub const FTYPE_SOCKET: u32 = 7;
pub const FTYPE_SEM: u32 = 8;
pub const FTYPE_PHOTON: u32 = 9;
pub const FTYPE_DUMPER: u32 = 10;
pub const FTYPE_MOUNT: u32 = 11;
pub const FTYPE_NAME: u32 = 12;
pub const FTYPE_TYMEM: u32 = 13;

/// The path manager's own requests (`<sys/pathmsg.h>`), from
/// `_PATHMGR_BASE`: a [`Connect`] header of this type, then an absolute
/// path. `_PATH_CHDIR` sets the working directory; `_PATH_OPENDIR`
/// turns a handle into an open directory.
pub const PATH_RESOLVE: u16 = 0x70;
pub const PATH_CHDIR: u16 = 0x71;
pub const PATH_CHROOT: u16 = 0x72;
pub const PATH_OPENDIR: u16 = 0x74;

pod!(Connect = 40, LinkReply = 20, Entry = 32, FtypeReply = 8);

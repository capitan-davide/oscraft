//! The I/O messages (`<sys/iomsg.h>`): what a client sends on a
//! connection to an open file, after `_IO_CONNECT` opened it. Each
//! starts with its type and, in a combined message, its length.

use crate::event::Sigevent;
use crate::info::{Fdinfo, MsgInfo32, MsgInfoHdr};
use crate::pod::pod;

/// The message types, from `_IO_BASE`.
pub const IO_CONNECT: u16 = 0x100;
pub const IO_READ: u16 = 0x101;
pub const IO_WRITE: u16 = 0x102;
pub const IO_RSVD_CLOSE_OCB: u16 = 0x103;
pub const IO_STAT: u16 = 0x104;
pub const IO_NOTIFY: u16 = 0x105;
pub const IO_DEVCTL: u16 = 0x106;
pub const IO_RSVD_UNBLOCK: u16 = 0x107;
pub const IO_PATHCONF: u16 = 0x108;
pub const IO_LSEEK: u16 = 0x109;
pub const IO_CHMOD: u16 = 0x10a;
pub const IO_CHOWN: u16 = 0x10b;
pub const IO_UTIME: u16 = 0x10c;
pub const IO_OPENFD: u16 = 0x10d;
pub const IO_FDINFO: u16 = 0x10e;
pub const IO_LOCK: u16 = 0x10f;
pub const IO_SPACE: u16 = 0x110;
pub const IO_SHUTDOWN: u16 = 0x111;
pub const IO_MMAP: u16 = 0x112;
pub const IO_MSG: u16 = 0x113;
pub const IO_RSVD_CHECK_MSG: u16 = 0x114;
pub const IO_DUP: u16 = 0x115;
pub const IO_CLOSE: u16 = 0x116;
pub const IO_RSVD_LOCK_OCB: u16 = 0x117;
pub const IO_RSVD_UNLOCK_OCB: u16 = 0x118;
pub const IO_SYNC: u16 = 0x119;
pub const IO_POWER: u16 = 0x11a;
pub const IO_ACL: u16 = 0x11b;
pub const IO_READ64: u16 = 0x11e;
pub const IO_WRITE64: u16 = 0x11f;
pub const IO_UTIME64: u16 = 0x121;
pub const IO_MAX: u16 = 0x1ff;

/// `_IO_COMBINE_FLAG`: in `combine_len`, another message follows.
pub const IO_COMBINE_FLAG: u16 = 0x8000;

/// `_IO_XTYPE_*`: a read's or write's extended type.
pub const IO_XTYPE_NONE: u32 = 0;
pub const IO_XTYPE_READCOND: u32 = 1;
pub const IO_XTYPE_OFFSET: u32 = 5;

/// `_IO_FLAG_RD` and `_IO_FLAG_WR`: an open's access, in `ioflag`.
pub const IO_FLAG_RD: u32 = 1;
pub const IO_FLAG_WR: u32 = 2;

/// The header every I/O message has, `struct _io_combine`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Header {
    pub r#type: u16,
    pub combine_len: u16,
}

/// `struct _io_read` and `struct _io_read64`, `struct _io_write` and
/// `struct _io_write64`: the same layout; the 64-bit forms put the
/// count's high word where the others have zero. An `_IO_XTYPE_OFFSET`
/// request is followed by the 64-bit offset (`struct _xtype_offset`).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct ReadWrite {
    pub r#type: u16,
    pub combine_len: u16,
    pub nbytes: u32,
    pub xtype: u32,
    pub nbytes_hi: u32,
}

/// `struct _io_stat`; the reply is a `struct stat` of `format`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Stat {
    pub r#type: u16,
    pub combine_len: u16,
    pub format: u32,
}

/// `struct _io_notify` and its reply's extended fields, which servers
/// that know the extension update in place.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Notify {
    pub r#type: u16,
    pub combine_len: u16,
    /// The action; in the reply, the conditions that hold.
    pub action: i32,
    pub flags: u32,
    pub zero: u32,
    pub event: Sigevent,
    pub mgr: [i32; 2],
    pub flags_extra_mask: u32,
    pub flags_exten: u32,
    pub nfds: i32,
    pub fd_first: i32,
    pub nfds_ready: i32,
    pub zero2: u32,
    pub timo: i64,
}

/// `_NOTIFY_ACTION_*`.
pub const NOTIFY_ACTION_TRANARM: i32 = 0;
pub const NOTIFY_ACTION_CONDARM: i32 = 1;
pub const NOTIFY_ACTION_POLL: i32 = 2;
pub const NOTIFY_ACTION_POLLARM: i32 = 3;
pub const NOTIFY_ACTION_EDGEARM: i32 = 4;
/// `_NOTIFY_COND_EXTEN`, and where the basic conditions are.
pub const NOTIFY_COND_EXTEN: u32 = 0x8000_0000;
pub const NOTIFY_COND_SHIFT: u32 = 28;

/// `struct _io_devctl`, followed by the data.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Devctl {
    pub r#type: u16,
    pub combine_len: u16,
    pub dcmd: i32,
    pub nbytes: u32,
    pub zero: i32,
}

/// `struct _io_devctl_reply`, followed by the data.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct DevctlReply {
    pub zero: u32,
    pub ret_val: i32,
    pub nbytes: u32,
    pub zero2: i32,
}

/// `struct _io_pathconf`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Pathconf {
    pub r#type: u16,
    pub combine_len: u16,
    pub name: i16,
    pub zero: u16,
}

/// `struct _io_lseek`; the reply is the new offset.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Lseek {
    pub r#type: u16,
    pub combine_len: u16,
    pub whence: i16,
    pub flags: u16,
    pub offset: u64,
}

/// `struct _io_chmod`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Chmod {
    pub r#type: u16,
    pub combine_len: u16,
    pub mode: u32,
}

/// `struct _io_chown`: the group before the owner.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Chown {
    pub r#type: u16,
    pub combine_len: u16,
    pub gid: i32,
    pub uid: i32,
}

/// `struct _io_utime`: 32-bit times, or now with `cur_flag`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Utime {
    pub r#type: u16,
    pub combine_len: u16,
    pub cur_flag: i32,
    pub actime: u32,
    pub modtime: u32,
}

/// `struct _io_utime64`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Utime64 {
    pub r#type: u16,
    pub combine_len: u16,
    pub zero: i32,
    pub atime_s: i64,
    pub atime_ns: i64,
    pub mtime_s: i64,
    pub mtime_ns: i64,
}

/// `struct _io_openfd`: a new open of the same file, for a client.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Openfd {
    pub r#type: u16,
    pub combine_len: u16,
    pub ioflag: u32,
    pub sflag: u16,
    pub xtype: u16,
    pub info: MsgInfoHdr,
    pub handle: u64,
}

/// `_IO_OPENFD_*`.
pub const IO_OPENFD_NONE: u16 = 0;
pub const IO_OPENFD_PIPE: u16 = 1;

/// `struct _io_fdinfo`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FdinfoReq {
    pub r#type: u16,
    pub combine_len: u16,
    pub flags: u32,
    pub path_len: u32,
    pub reserved: u32,
}

/// `struct _io_fdinfo_reply`, followed by the path.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct FdinfoReply {
    pub zero: [u32; 2],
    pub info: Fdinfo,
}

/// `struct _io_lock`, followed by a `flock_t`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Lock {
    pub r#type: u16,
    pub combine_len: u16,
    pub subtype: u32,
    pub nbytes: u32,
}

/// `struct _io_lock_reply`, followed by a `flock_t` for `F_GETLK`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct LockReply {
    pub zero: [u32; 3],
}

/// `flock_t`: a record lock.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Flock {
    pub l_type: i16,
    pub l_whence: i16,
    pub l_zero1: i32,
    pub l_start: i64,
    pub l_len: i64,
    pub l_pid: i32,
    pub l_sysid: u32,
}

/// The lock commands (`<fcntl.h>`), `struct _io_lock`'s subtype, with
/// a 64-bit `flock_t`.
pub const F_SETLK: u32 = 106;
pub const F_SETLKW: u32 = 107;
pub const F_GETLK: u32 = 114;
/// `flock_t`'s `l_type`; with `F_FLOCK`, a lock of the open file rather
/// than of the process.
pub const F_RDLCK: i16 = 1;
pub const F_WRLCK: i16 = 2;
pub const F_UNLCK: i16 = 3;
pub const F_EXLCK: i16 = 4;
pub const F_FLOCK: i16 = 0x80;

/// `struct _io_space`; the reply is the new size.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Space {
    pub r#type: u16,
    pub combine_len: u16,
    pub subtype: u16,
    pub whence: i16,
    pub start: u64,
    pub len: u64,
}

/// `struct _io_mmap`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Mmap {
    pub r#type: u16,
    pub combine_len: u16,
    pub prot: u32,
    pub offset: u64,
    pub info: MsgInfo32,
    pub required_prot: u32,
    pub zero: [u32; 3],
    pub requested_len: u64,
}

/// `struct _io_mmap_reply`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct MmapReply {
    pub zero: u32,
    pub allowed_prot: u32,
    pub offset: u64,
    pub coid: i32,
    pub fd: i32,
}

/// `struct _io_dup`: the client's connection, for the server to know
/// as its own.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Dup {
    pub r#type: u16,
    pub combine_len: u16,
    pub info: MsgInfoHdr,
    pub handle: u64,
}

/// `struct _io_msg`: a server's private message, by manager.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Msg {
    pub r#type: u16,
    pub combine_len: u16,
    pub mgrid: u16,
    pub subtype: u16,
}

/// `struct _io_sync`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Sync {
    pub r#type: u16,
    pub combine_len: u16,
    pub flag: u32,
}

/// `struct _io_acl`, followed by an ACL (`struct _acl_header` and its
/// entries).
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Acl {
    pub r#type: u16,
    pub combine_len: u16,
    pub subtype: u32,
    pub zero: [i32; 2],
}

/// `struct _io_acl_reply`, followed by the ACL.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct AclReply {
    pub zero: [u32; 4],
}

/// `_IO_ACL_*`: what an `_IO_ACL` message asks.
pub const IO_ACL_GET: u32 = 0;
pub const IO_ACL_SET: u32 = 1;
pub const IO_ACL_CHK: u32 = 2;

/// `struct _acl_header` (`<sys/ioacl.h>`): an ACL's size in bytes, with
/// the header, its type and its number of entries.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct AclHeader {
    pub size: u32,
    pub r#type: i16,
    pub nents: u16,
}

/// `struct _acl_posix_ent` (`<sys/acl_posix.h>`): the tag in the top
/// byte, the permissions below; and the user or group.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct AclEntry {
    pub tag_permset: u32,
    pub id: i32,
}

pod!(
    Header = 4,
    ReadWrite = 16,
    Stat = 8,
    Notify = 0x58,
    Devctl = 16,
    DevctlReply = 16,
    Pathconf = 8,
    Lseek = 16,
    Chmod = 8,
    Chown = 12,
    Utime = 16,
    Utime64 = 0x28,
    Openfd = 48,
    FdinfoReq = 16,
    FdinfoReply = 72,
    Lock = 12,
    LockReply = 12,
    Flock = 32,
    Space = 24,
    Mmap = 88,
    MmapReply = 24,
    Dup = 40,
    Msg = 8,
    Sync = 8,
    Acl = 16,
    AclReply = 16,
    AclHeader = 8,
    AclEntry = 8,
);

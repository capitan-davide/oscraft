//! The open flags as FreeBSD numbers them (`<sys/fcntl.h>`), what the
//! kernel and native servers speak, and how they map to QNX's
//! ([`fcntl`](crate::qnx_fcntl)). `_IO_CONNECT`'s `ioflag` is FreeBSD's
//! `FFLAGS()` form, the access plus one, with `O_EXEC` a bit of its own.
//! QNX's protocol bits, which FreeBSD has no number for, sit where it
//! doesn't use any.

#![allow(missing_docs)]

pub use crate::qnx_fcntl::{SH_COMPAT, SH_DENYNO, SH_DENYRD, SH_DENYRW, SH_DENYWR, SH_DOS, SH_MASK};

pub const O_RDONLY: u32 = 0x0;
pub const O_WRONLY: u32 = 0x1;
pub const O_RDWR: u32 = 0x2;
pub const O_ACCMODE: u32 = 0x3;
pub const O_NONBLOCK: u32 = 0x4;
pub const O_APPEND: u32 = 0x8;
pub const O_ASYNC: u32 = 0x40;
pub const O_SYNC: u32 = 0x80;
pub const O_NOFOLLOW: u32 = 0x100;
pub const O_CREAT: u32 = 0x200;
pub const O_TRUNC: u32 = 0x400;
pub const O_EXCL: u32 = 0x800;
pub const O_NOCTTY: u32 = 0x8000;
pub const O_DIRECTORY: u32 = 0x2_0000;
pub const O_EXEC: u32 = 0x4_0000;
pub const O_CLOEXEC: u32 = 0x10_0000;
pub const O_DSYNC: u32 = 0x100_0000;
// QNX's protocol bits.
pub const O_PERMCHECK: u32 = 0x1000;
pub const O_ANON: u32 = 0x2000;
pub const O_RSYNC: u32 = 0x800_0000;
pub const O_LARGEFILE: u32 = 0x1000_0000;
pub const O_DIRFD: u32 = 0x2000_0000;
pub const O_REALIDS: u32 = 0x4000_0000;
pub const O_NOSYMLINK: u32 = 0x8000_0000;

/// What `fcntl(F_GETFL)` and `F_SETFL` carry.
pub const STATUS: u32 = O_NONBLOCK | O_APPEND | O_ASYNC | O_SYNC | O_DSYNC | O_RSYNC | O_LARGEFILE;

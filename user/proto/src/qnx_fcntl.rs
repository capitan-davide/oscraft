//! The flags and commands of `<fcntl.h>` and `<share.h>`: what an
//! open asks for, as clients pass them and servers read them.

/// Open flags (`O_*`): the access mode in the low bits, then the options.
pub const O_ACCMODE: u32 = 0x7;
pub const O_ANON: u32 = 0x40000;
pub const O_APPEND: u32 = 0x8;
pub const O_ASYNC: u32 = 0x10000;
pub const O_BINARY: u32 = 0x0;
pub const O_CLOEXEC: u32 = 0x2000;
pub const O_CREAT: u32 = 0x100;
pub const O_DIRECTORY: u32 = 0x100000;
pub const O_DIRFD: u32 = 0x200000;
pub const O_DSYNC: u32 = 0x10;
pub const O_EXCL: u32 = 0x400;
pub const O_EXEC: u32 = 0x3;
pub const O_LARGEFILE: u32 = 0x8000;
pub const O_NOCTTY: u32 = 0x800;
pub const O_NOFOLLOW: u32 = 0x1000;
pub const O_NONBLOCK: u32 = 0x80;
pub const O_NOSYMLINK: u32 = 0x20000;
pub const O_PERMCHECK: u32 = 0x80000;
pub const O_RDONLY: u32 = 0x0;
pub const O_RDWR: u32 = 0x2;
pub const O_REALIDS: u32 = 0x4000;
pub const O_RSYNC: u32 = 0x40;
pub const O_SYNC: u32 = 0x20;
pub const O_TEXT: u32 = 0x0;
pub const O_TRUNC: u32 = 0x200;
pub const O_WRONLY: u32 = 0x1;

/// Sharing modes (`<share.h>`), a connect request's `sflag`.
pub const SH_COMPAT: u16 = 0x0;
pub const SH_DENYNO: u16 = 0x40;
pub const SH_DENYRD: u16 = 0x30;
pub const SH_DENYRW: u16 = 0x10;
pub const SH_DENYWR: u16 = 0x20;
pub const SH_DOS: u16 = 0x1;
pub const SH_MASK: u16 = 0x70;

/// `fcntl()`'s commands; the lock commands and types are in [`io`](crate::io).
pub const F_BLKSIZE: i32 = 0x13;
pub const F_BLOCKS: i32 = 0x12;
pub const F_CHKFL: i32 = 0x8;
pub const F_DUP2FD: i32 = 0x25;
pub const F_DUPFD: i32 = 0x0;
pub const F_GETFD: i32 = 0x1;
pub const F_GETFL: i32 = 0x3;
pub const F_GETOWN: i32 = 0x23;
pub const F_ISSTREAM: i32 = 0xd;
pub const F_NPRIV: i32 = 0x10;
pub const F_PRIV: i32 = 0xf;
pub const F_QUOTACTL: i32 = 0x11;
pub const F_RGETLK: i32 = 0x15;
pub const F_RSETLK: i32 = 0x14;
pub const F_RSETLKW: i32 = 0x16;
pub const F_SETFD: i32 = 0x2;
pub const F_SETFL: i32 = 0x4;
pub const F_SETOWN: i32 = 0x24;

/// The descriptor flag.
pub const FD_CLOEXEC: i32 = 0x1;

/// The `*at()` functions' flags.
pub const AT_EACCESS: i32 = 0x1;
pub const AT_REMOVEDIR: i32 = 0x8;
pub const AT_SYMLINK_FOLLOW: i32 = 0x4;
pub const AT_SYMLINK_NOFOLLOW: i32 = 0x2;

/// The `*at()` functions' descriptor for the working directory.
pub const AT_FDCWD: i32 = -100;

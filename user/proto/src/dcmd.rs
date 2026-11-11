//! Device commands (`<sys/dcmd_*.h>`) and the data some of them carry.
//! A command's number holds the size of its data and whether it goes to
//! the server, comes back, or both.

use crate::pod::pod;

/// `__DIOF`, `__DIOT`, `__DIOTF`: the direction bits.
pub const DIOF: u32 = 0x4000_0000;
pub const DIOT: u32 = 0x8000_0000;

/// The size of a command's data.
pub const fn size(dcmd: u32) -> usize {
    (dcmd >> 16 & 0x3fff) as usize
}

/// Every server's (`<sys/dcmd_all.h>`).
pub const ALL_GETFLAGS: u32 = 0x4004_0101;
pub const ALL_SETFLAGS: u32 = 0x8004_0102;
pub const ALL_GETOWN: u32 = 0x4004_0104;
pub const ALL_SETOWN: u32 = 0x8004_0105;
pub const ALL_GETMOUNTFLAGS: u32 = 0x4004_0103;
pub const ALL_FADVISE: u32 = 0x8018_0106;

/// File systems' (`<sys/dcmd_blk.h>`): the status `statvfs()` reports.
pub const FSYS_STATVFS: u32 = 0x40a8_020d;

/// `struct __msg_statvfs` (`<sys/statvfs.h>`): `statvfs`'s fields as a
/// server sends them, the counts in 64 bits and the rest in 32.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MsgStatvfs {
    pub bsize: u32,
    pub frsize: u32,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub favail: u64,
    pub fsid: u32,
    pub basetype: [u8; 16],
    pub flag: u32,
    pub namemax: u32,
    pub filler: [u32; 21],
}

/// `struct statvfs` (and `statvfs64`, the same here).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Statvfs {
    pub bsize: u64,
    pub frsize: u64,
    pub blocks: u64,
    pub bfree: u64,
    pub bavail: u64,
    pub files: u64,
    pub ffree: u64,
    pub favail: u64,
    pub fsid: u64,
    pub basetype: [u8; 16],
    pub flag: u64,
    pub namemax: u64,
    pub filler: [u32; 21],
}

/// Terminals' (`<sys/dcmd_chr.h>`), and `<sys/ioctl.h>`'s that are the
/// same.
pub const CHR_ISATTY: u32 = 0x318;
pub const CHR_GETSID: u32 = 0x4004_0307;
pub const CHR_TTYINFO: u32 = 0x4024_030a;
pub const CHR_SERCTL: u32 = 0x8004_0314;
pub const CHR_TCINJECTC: u32 = 0x8004_0316;
pub const CHR_TCINJECTR: u32 = 0x8004_0317;
pub const CHR_GRANTPT: u32 = 0x32a;
pub const TIOCGETA: u32 = 0x404c_7413;
pub const TIOCSETA: u32 = 0x804c_7414;
pub const TIOCSETAW: u32 = 0x804c_7415;
pub const TIOCSETAF: u32 = 0x804c_7416;
pub const TIOCFLUSH: u32 = 0x8004_7410;
pub const TIOCDRAIN: u32 = 0x745e;
pub const TIOCGPGRP: u32 = 0x4004_7477;
pub const TIOCSPGRP: u32 = 0x8004_7476;
pub const TIOCGWINSZ: u32 = 0x4008_7468;
pub const TIOCSWINSZ: u32 = 0x8008_7467;
pub const TIOCMGET: u32 = 0x4004_746a;
pub const TCXONC: u32 = 0x8004_5406;
pub const FIONREAD: u32 = 0x4004_667f;

/// `<sys/dcmd_misc.h>`'s: a command's embedded pointers.
pub const MISC_GETPTREMBED: u32 = 0xc010_0503;

/// QNX's `struct termios`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QnxTermios {
    pub c_iflag: u32,
    pub c_oflag: u32,
    pub c_cflag: u32,
    pub c_lflag: u32,
    pub c_cc: [u8; 40],
    pub reserved: [u32; 3],
    pub c_ispeed: u32,
    pub c_ospeed: u32,
}

/// `struct winsize`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Winsize {
    pub ws_row: u16,
    pub ws_col: u16,
    pub ws_xpixel: u16,
    pub ws_ypixel: u16,
}

/// `struct __ioctl_getptrembed`: the command whose pointers the server
/// asks for, and how many buffers of them it may name.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct GetPtrEmbed {
    pub dcmd: u32,
    pub nbytes: u32,
    pub niov: u32,
    pub zero: u32,
}

pod!(
    QnxTermios = 0x4c,
    Winsize = 8,
    GetPtrEmbed = 16,
    MsgStatvfs = 0xa8,
    Statvfs = 0xc0
);

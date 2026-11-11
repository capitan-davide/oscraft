//! `struct stat` (`<sys/stat.h>`) in QNX OS 8.0's preferred form, and
//! the file types and forms around it.

use crate::pod::pod;

/// `struct stat`, form `_STAT_FORM_T64_2008`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct QnxStat {
    pub st_ino: u64,
    pub st_size: i64,
    pub st_dev: u32,
    pub st_rdev: u32,
    pub st_uid: u32,
    pub st_gid: u32,
    /// The 32-bit times of older forms: modification, access, change.
    pub old_times: [u32; 3],
    pub st_mode: u32,
    pub st_nlink: u32,
    pub st_blocksize: u32,
    pub st_nblocks: i32,
    pub st_blksize: u32,
    pub st_blocks: i64,
    pub st_mtim: [i64; 2],
    pub st_atim: [i64; 2],
    pub st_ctim: [i64; 2],
}

pod!(QnxStat = 0x78);

/// `_STAT_FORM_*`: what a server's `_IO_STAT` reply is in.
pub const FORM_UNSET: u32 = 0;
pub const FORM_T32_2001: u32 = 1;
pub const FORM_T32_2008: u32 = 2;
pub const FORM_T64_2008: u32 = 3;
pub const FORM_PREFERRED: u32 = FORM_T64_2008;

/// The file types, in `st_mode`.
pub const S_IFMT: u32 = 0xf000;
pub const S_IFIFO: u32 = 0x1000;
pub const S_IFCHR: u32 = 0x2000;
pub const S_IFDIR: u32 = 0x4000;
pub const S_IFNAM: u32 = 0x5000;
pub const S_IFBLK: u32 = 0x6000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFLNK: u32 = 0xa000;
pub const S_IFSOCK: u32 = 0xc000;

/// FreeBSD's `struct stat`.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: u16,
    pub st_bsdflags: i16,
    pub st_uid: u32,
    pub st_gid: u32,
    pub st_padding1: i32,
    pub st_rdev: u64,
    pub st_atim: [i64; 2],
    pub st_mtim: [i64; 2],
    pub st_ctim: [i64; 2],
    pub st_birthtim: [i64; 2],
    pub st_size: i64,
    pub st_blocks: i64,
    pub st_blksize: i32,
    pub st_flags: u32,
    pub st_gen: u64,
    pub st_filerev: u64,
    pub st_spare: [u64; 9],
}

pod!(Stat = 224);

/// The form of a reply in FreeBSD's `struct stat`, as native servers and
/// process 1 answer a FreeBSD process.
pub const FORM_FREEBSD: u32 = 0x4642;

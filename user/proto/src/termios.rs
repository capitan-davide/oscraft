//! `struct termios`, FreeBSD's and QNX's, and from one to the other: for
//! libsys, on QNX's drivers, and for `qnxd`, on native servers.

use crate::pod::pod;

/// QNX's `struct termios`: 76 bytes, 40 control characters, three
/// reserved words before the speeds.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct QnxTermios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub cc: [u8; 40],
    pub reserved: [u32; 3],
    pub ispeed: u32,
    pub ospeed: u32,
}

impl Default for QnxTermios {
    fn default() -> Self {
        QnxTermios { iflag: 0, oflag: 0, cflag: 0, lflag: 0, cc: [0; 40], reserved: [0; 3], ispeed: 0, ospeed: 0 }
    }
}

/// FreeBSD's: 44 bytes, 20 control characters.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Termios {
    pub iflag: u32,
    pub oflag: u32,
    pub cflag: u32,
    pub lflag: u32,
    pub cc: [u8; 20],
    pub ispeed: u32,
    pub ospeed: u32,
}

const _: () = assert!(size_of::<QnxTermios>() == 76 && size_of::<Termios>() == 44);

pod!(QnxTermios = 76);
pod!(Termios = 44);

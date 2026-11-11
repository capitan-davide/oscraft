//! `<sys/ioctl.h>`'s commands that are not device commands as they are:
//! the descriptor's flags, the old terminal interfaces (`sgttyb`,
//! `tchars`, `ltchars`, System V's `termio`), the modem lines. The C
//! library translates them.

pub const FIOCLEX: u32 = 0x6601;
pub const FIONCLEX: u32 = 0x6602;
pub const FIOGETOWN: u32 = 0x4004_667b;
pub const FIOSETOWN: u32 = 0x8004_667c;
pub const FIOASYNC: u32 = 0x8004_667d;
pub const FIONBIO: u32 = 0x8004_667e;
pub const TIOCHPCL: u32 = 0x7402;
pub const TIOCGETP: u32 = 0x4008_7408;
pub const TIOCSETP: u32 = 0x8008_7409;
pub const TIOCSETN: u32 = 0x8008_740a;
pub const TIOCEXCL: u32 = 0x740d;
pub const TIOCNXCL: u32 = 0x740e;
pub const TIOCSETC: u32 = 0x8006_7411;
pub const TIOCGETC: u32 = 0x4006_7412;
pub const TIOCSCTTY: u32 = 0x7461;
pub const TIOCMBIC: u32 = 0x8004_746b;
pub const TIOCMBIS: u32 = 0x8004_746c;
pub const TIOCMSET: u32 = 0x8004_746d;
pub const TIOCSTART: u32 = 0x746e;
pub const TIOCSTOP: u32 = 0x746f;
pub const TIOCNOTTY: u32 = 0x7471;
pub const TIOCSTI: u32 = 0x8001_7472;
pub const TIOCGLTC: u32 = 0x4006_7474;
pub const TIOCSLTC: u32 = 0x8006_7475;
pub const TIOCCDTR: u32 = 0x7478;
pub const TIOCSDTR: u32 = 0x7479;
pub const TIOCCBRK: u32 = 0x747a;
pub const TIOCLGET: u32 = 0x4004_747c;
pub const TIOCLSET: u32 = 0x8004_747d;
pub const TIOCSETPGRP: u32 = 0x8004_7482;
pub const TIOCGETPGRP: u32 = 0x4004_7483;
pub const TCGETA: u32 = 0x402e_5401;
pub const TCSETA: u32 = 0x802e_5402;
pub const TCSETAW: u32 = 0x802e_5403;
pub const TCSETAF: u32 = 0x802e_5404;
pub const TCSBRK: u32 = 0x8004_5405;
pub const TCFLSH: u32 = 0x8004_5407;
pub const TCGETS: u32 = 0x404c_540d;
pub const TCSETS: u32 = 0x804c_540e;
pub const TCSETSW: u32 = 0x804c_540f;
pub const TCSETSF: u32 = 0x804c_5410;

/// `struct sgttyb` (`<sgtty.h>`): speeds as indices, the erase and kill
/// characters, and the flags.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Sgttyb {
    pub sg_ispeed: u8,
    pub sg_ospeed: u8,
    pub sg_erase: u8,
    pub sg_kill: u8,
    pub sg_flags: i32,
}

/// `struct tchars` and `struct ltchars`: six special characters each.
pub type Tchars = [u8; 6];
pub type Ltchars = [u8; 6];

/// `struct termio` (`<sys/termio.h>`), System V's.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Termio {
    pub c_iflag: u16,
    pub c_oflag: u16,
    pub c_cflag: u16,
    pub c_lflag: u16,
    pub c_line: u8,
    pub c_padding: u8,
    pub c_cc: [u16; 18],
}

crate::pod::pod!(Sgttyb = 8, Termio = 0x2e);

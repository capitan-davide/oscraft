//! The QNX personality, the compatibility one, for the SDP's unmodified
//! binaries. Its kernel ABI is procnto's.

use crate::ker::errno::{EIO, QNX_ONLY};

// ANCHOR: errno
/// What a QNX process sees for the kernel's error `e`. A FreeBSD error
/// QNX has no name for is EIO.
pub fn errno_out(e: u32) -> u32 {
    if e & QNX_ONLY != 0 {
        return e & !QNX_ONLY;
    }
    qnxabi::errno::to_qnx(e as i32).map_or(EIO, |q| q as u32)
}

/// The kernel's number for QNX's error `e`, as a QNX server hands it to
/// `MsgError()`.
pub fn errno_in(e: u32) -> u32 {
    qnxabi::errno::from_qnx(e as i32).map_or(e | QNX_ONLY, |b| b as u32)
}
// ANCHOR_END: errno

use crate::ker::signal::{QNX_SET, SigInfo, SigSet};
use proto::termios::{Termios, QnxTermios};

// ANCHOR: signals
/// (QNX, kernel) signal numbers. The kernel's are FreeBSD's. QNX's own
/// signals have numbers FreeBSD never uses (34 to 64), so they take
/// nothing from it.
const SIGNALS: &[(usize, usize)] = &[
    (16, 30), (17, 31), (18, 20), (19, 47), (20, 28), (21, 16), (22, 23), (23, 17), (24, 18),
    (25, 19), (26, 21), (27, 22), (28, 26), (29, 27), (30, 24), (31, 25), (32, 41), (33, 42),
    (34, 29), (35, 32), (36, 33), (37, 43), (38, 44), (39, 45), (40, 46),
];
/// QNX's real-time signals (41 to 56) start at FreeBSD's `SIGRTMIN`.
const RT_QNX: usize = 41;
const RT_KERNEL: usize = 65;
const RT_COUNT: usize = 16;

/// The kernel's number for QNX's signal `q`.
pub fn signo_in(q: usize) -> usize {
    match q {
        16..=40 => SIGNALS.iter().find(|p| p.0 == q).map_or(q, |p| p.1),
        41..=56 => q - RT_QNX + RT_KERNEL,
        _ => q,
    }
}

/// QNX's number for the kernel's signal `s`, if it has one.
pub fn signo_out(s: usize) -> Option<usize> {
    match s {
        1..=15 | 57..=64 => Some(s),
        65..=80 => Some(s - RT_KERNEL + RT_QNX),
        _ => SIGNALS.iter().find(|p| p.1 == s).map(|p| p.0),
    }
}
// ANCHOR_END: signals

/// The kernel's set for QNX's.
pub fn set_in(q: u64) -> SigSet {
    (1..=64).filter(|n| q >> (n - 1) & 1 != 0).fold(0, |set, n| set | 1 << (signo_in(n) - 1))
}

/// QNX's set for the kernel's.
pub fn set_out(set: SigSet) -> u64 {
    (1..128)
        .filter(|n| set >> (n - 1) & 1 != 0)
        .filter_map(signo_out)
        .fold(0, |q, n| q | 1 << (n - 1))
}

/// The signal of `ready` a QNX process takes first: QNX's lowest number
/// (procnto, measured).
pub fn next(ready: SigSet) -> usize {
    (1..=64)
        .map(signo_in)
        .find(|&s| ready >> (s - 1) & 1 != 0)
        .unwrap_or(ready.trailing_zeros() as usize + 1)
}

/// (QNX, FreeBSD) `si_code`s of senders: `SI_USER`, `SI_QUEUE`, `SI_TIMER`,
/// `SI_ASYNCIO`, `SI_MESGQ`.
const SI_CODES: [(i32, i32); 5] =
    [(0, 0x10001), (-2, 0x10002), (-3, 0x10003), (-4, 0x10004), (-5, 0x10005)];

/// The kernel's `si_code` for QNX's. QNX's others stay as they are.
pub fn code_in(c: i32) -> i32 {
    SI_CODES.iter().find(|p| p.0 == c).map_or(c, |p| p.1)
}

/// QNX's `si_code` for the kernel's.
pub fn code_out(c: i32) -> i32 {
    SI_CODES.iter().find(|p| p.1 == c).map_or(c, |p| p.0)
}

/// (QNX, FreeBSD) `sa_flags`.
const SA_FLAGS: [(u32, u32); 6] = [(0x1, 0x8), (0x2, 0x40), (0x4, 0x4), (0x8, 0x1), (0x10, 0x10), (0x20, 0x20)];

/// The kernel's `sa_flags` for QNX's.
pub fn sa_flags_in(q: u32) -> u32 {
    SA_FLAGS.iter().filter(|p| q & p.0 != 0).fold(0, |f, p| f | p.1)
}

/// QNX's `sa_flags` for the kernel's.
pub fn sa_flags_out(f: u32) -> u32 {
    SA_FLAGS.iter().filter(|p| f & p.1 != 0).fold(0, |q, p| q | p.0)
}

/// `info` with QNX's numbers: the signal, and a child's status if it is
/// a signal.
pub fn view(info: SigInfo) -> SigInfo {
    let status = match (info.signo as usize, info.code) {
        (20, 2..=6) => signo_out(info.status as usize).map_or(info.status, |s| s as i32),
        _ => info.status,
    };
    SigInfo {
        signo: signo_out(info.signo as usize).unwrap_or(0) as i32,
        code: code_out(info.code),
        status,
        ..info
    }
}

const _: () = assert!(QNX_SET == u64::MAX as SigSet && RT_COUNT == 16);

pub use qnxabi::stat::{EXTRA as STAT_EXTRA, Extra as StatExtra};

/// QNX's `struct stat`, form `_STAT_FORM_T64_2008`, as bytes.
pub fn stat_out(st: &proto::stat::Stat, extra: StatExtra) -> [u8; 120] {
    let q = qnxabi::stat::to_qnx(st, extra);
    // SAFETY: `Stat` is 120 plain bytes, `proto::stat` asserts its size.
    unsafe { core::mem::transmute(q) }
}

/// QNX's `struct dirent` for a name: `d_ino`, `d_offset`, `d_reclen`,
/// `d_namelen`, the name with its NUL at 20, padded to 8 bytes.
pub fn dirent(ino: u64, off: u64, name: &str) -> alloc::vec::Vec<u8> {
    let reclen = (20 + name.len() + 1).next_multiple_of(8);
    let mut r = alloc::vec![0u8; reclen];
    r[0..8].copy_from_slice(&ino.to_le_bytes());
    r[8..16].copy_from_slice(&off.to_le_bytes());
    r[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
    r[18..20].copy_from_slice(&(name.len() as u16).to_le_bytes());
    r[20..20 + name.len()].copy_from_slice(name.as_bytes());
    r
}

/// FreeBSD's `struct termios` (the kernel's) for QNX's.
pub fn termios_in(q: &[u8; 76]) -> [u8; 44] {
    // SAFETY: plain data of this size (`proto::termios`).
    let q: QnxTermios = unsafe { core::mem::transmute(*q) };
    let t = qnxabi::termios::from_qnx(&q);
    // SAFETY: as above.
    unsafe { core::mem::transmute(t) }
}

/// QNX's `struct termios` for the kernel's: `base` keeps the characters
/// FreeBSD has no slot for.
pub fn termios_out(t: &[u8; 44], base: &[u8; 76]) -> [u8; 76] {
    // SAFETY: plain data of this size (`proto::termios`).
    let t: Termios = unsafe { core::mem::transmute(*t) };
    // SAFETY: as above.
    let base: QnxTermios = unsafe { core::mem::transmute(*base) };
    let q = qnxabi::termios::to_qnx(&t, &base);
    // SAFETY: as above.
    unsafe { core::mem::transmute(q) }
}

pub use qnxabi::fcntl::{flags_to_qnx as status_out, ioflag_from_qnx as ioflag_in, ioflag_to_qnx as ioflag_out};

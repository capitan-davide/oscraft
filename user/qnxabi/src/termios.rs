//! Terminal settings between QNX's `struct termios` and FreeBSD's.

use proto::termios::{Termios, QnxTermios};

// (QNX, FreeBSD) flag pairs, per word; the ones both have.
const IFLAGS: [(u32, u32); 13] = [
    (0x1, 0x1), (0x2, 0x2), (0x4, 0x4), (0x8, 0x8), (0x10, 0x10), (0x20, 0x20), (0x40, 0x40),
    (0x80, 0x80), (0x100, 0x100), (0x400, 0x200), (0x1000, 0x400), (0x800, 0x800), (0x2000, 0x2000),
];
const OFLAGS: [(u32, u32); 5] = [(0x1, 0x1), (0x4, 0x2), (0x8, 0x10), (0x10, 0x20), (0x20, 0x40)];
const CFLAGS: [(u32, u32); 6] =
    [(0x40, 0x400), (0x80, 0x800), (0x100, 0x1000), (0x200, 0x2000), (0x400, 0x4000), (0x800, 0x8000)];
const LFLAGS: [(u32, u32); 11] = [
    (0x1, 0x80), (0x2, 0x100), (0x8, 0x8), (0x10, 0x2), (0x20, 0x4), (0x40, 0x10), (0x80, 0x8000_0000),
    (0x100, 0x40_0000), (0x8000, 0x400), (0x200, 0x40), (0x800, 0x1),
];
/// FreeBSD's control-character index for each of QNX's first 18, where
/// FreeBSD has one (VSWTCH it has not).
const CC_QNX_TO_BSD: [Option<usize>; 18] = [
    Some(8), Some(9), Some(3), Some(5), Some(0), Some(1), Some(2), None, Some(12), Some(13), Some(10),
    Some(11), Some(6), Some(15), Some(4), Some(14), Some(16), Some(17),
];

fn map(flags: u32, table: &[(u32, u32)], from_qnx: bool) -> u32 {
    let mut out = 0;
    for &(q, b) in table {
        let (from, to) = if from_qnx { (q, b) } else { (b, q) };
        if flags & from != 0 {
            out |= to;
        }
    }
    out
}

/// Character size: QNX's CS5..CS8 are 0, 0x10, 0x20, 0x30; FreeBSD's
/// 0, 0x100, 0x200, 0x300.
fn csize_to_bsd(q: u32) -> u32 {
    (q & 0x30) << 4
}

fn csize_to_qnx(b: u32) -> u32 {
    (b & 0x300) >> 4
}

/// FreeBSD's settings for QNX's.
pub fn from_qnx(q: &QnxTermios) -> Termios {
    let mut t = Termios {
        iflag: map(q.iflag, &IFLAGS, true),
        oflag: map(q.oflag, &OFLAGS, true),
        cflag: map(q.cflag, &CFLAGS, true) | csize_to_bsd(q.cflag),
        lflag: map(q.lflag, &LFLAGS, true),
        cc: [0; 20],
        ispeed: q.ispeed,
        ospeed: q.ospeed,
    };
    for (i, slot) in CC_QNX_TO_BSD.iter().enumerate() {
        if let Some(b) = slot {
            t.cc[*b] = q.cc[i];
        }
    }
    t
}

/// QNX's settings for FreeBSD's; `old` keeps the characters FreeBSD lacks.
pub fn to_qnx(b: &Termios, old: &QnxTermios) -> QnxTermios {
    let mut t = *old; // the characters FreeBSD lacks keep their values
    t.iflag = map(b.iflag, &IFLAGS, false);
    t.oflag = map(b.oflag, &OFLAGS, false);
    t.cflag = map(b.cflag, &CFLAGS, false) | csize_to_qnx(b.cflag);
    t.lflag = map(b.lflag, &LFLAGS, false);
    for (i, slot) in CC_QNX_TO_BSD.iter().enumerate() {
        if let Some(bi) = slot {
            t.cc[i] = b.cc[*bi];
        }
    }
    t.ispeed = b.ispeed;
    t.ospeed = b.ospeed;
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_go_there_and_back() {
        let mut q = QnxTermios::default();
        (q.iflag, q.oflag, q.cflag, q.lflag) = (0x400, 0x1, 0x30, 0x8);
        q.cc[0] = 3;
        q.ispeed = 115200;
        let back = to_qnx(&from_qnx(&q), &QnxTermios::default());
        assert_eq!((back.iflag, back.oflag, back.cflag, back.lflag), (0x400, 0x1, 0x30, 0x8));
        assert_eq!((back.cc[0], back.ispeed), (3, 115200));
    }

    #[test]
    fn what_freebsd_lacks_stays() {
        let mut old = QnxTermios::default();
        old.cc[7] = 9; // VSWTCH
        assert_eq!(to_qnx(&Termios::default(), &old).cc[7], 9);
    }
}

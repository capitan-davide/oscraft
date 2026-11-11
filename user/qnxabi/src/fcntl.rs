//! Open and status flags between QNX's numbers and FreeBSD's.

use proto::fcntl::*;

/// (QNX, FreeBSD).
const FLAGS: [(u32, u32); 19] = [
    (0x80, O_NONBLOCK), (0x8, O_APPEND), (0x1_0000, O_ASYNC), (0x20, O_SYNC), (0x10, O_DSYNC),
    (0x40, O_RSYNC), (0x100, O_CREAT), (0x200, O_TRUNC), (0x400, O_EXCL), (0x800, O_NOCTTY),
    (0x1000, O_NOFOLLOW), (0x10_0000, O_DIRECTORY), (0x2000, O_CLOEXEC), (0x8000, O_LARGEFILE),
    (0x20_0000, O_DIRFD), (0x4000, O_REALIDS), (0x2_0000, O_NOSYMLINK), (0x8_0000, O_PERMCHECK),
    (0x4_0000, O_ANON),
];

/// FreeBSD's flag bits for QNX's; the low bits stay.
pub fn flags_from_qnx(q: u32) -> u32 {
    FLAGS.iter().filter(|p| q & p.0 != 0).fold(q & 3, |b, p| b | p.1)
}

/// QNX's flag bits for FreeBSD's; the low bits stay.
pub fn flags_to_qnx(b: u32) -> u32 {
    FLAGS.iter().filter(|p| b & p.1 != 0).fold(b & 3, |q, p| q | p.0)
}

/// An `ioflag` of QNX's, whose low 3 bits are the access plus one, 4 for
/// `O_EXEC`.
pub fn ioflag_from_qnx(q: u32) -> u32 {
    flags_from_qnx(q & !7) | if q & 7 == 4 { O_EXEC } else { q & 3 }
}

/// QNX's `ioflag` for FreeBSD's.
pub fn ioflag_to_qnx(b: u32) -> u32 {
    flags_to_qnx(b & !(3 | O_EXEC)) | if b & O_EXEC != 0 { 4 } else { b & 3 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_flag_goes_there_and_back() {
        for &(qnx, bsd) in &FLAGS {
            assert_eq!(flags_from_qnx(qnx), bsd);
            assert_eq!(flags_to_qnx(bsd), qnx);
        }
    }

    #[test]
    fn an_ioflag_keeps_its_access() {
        for access in 1..=3 {
            assert_eq!(ioflag_to_qnx(ioflag_from_qnx(access | 0x100)), access | 0x100);
        }
        assert_eq!(ioflag_from_qnx(4), O_EXEC);
        assert_eq!(ioflag_to_qnx(O_EXEC), 4);
    }

    #[test]
    fn shm_anon_is_ours_in_freebsd_form() {
        assert_eq!(ioflag_from_qnx(0x4_0000) & O_ANON, O_ANON);
    }
}

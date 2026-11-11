//! `struct stat` between QNX's and FreeBSD's.

use proto::stat::{Stat, QnxStat};

/// What QNX's `struct stat` says that FreeBSD's doesn't: `st_blocksize`,
/// and mode bits above FreeBSD's 16 (`_S_TRUST_EXT`).
#[derive(Clone, Copy)]
pub struct Extra {
    pub blocksize: u32,
    pub mode_bits: u32,
}

pub const EXTRA: Extra = Extra { blocksize: 512, mode_bits: 0 };

/// FreeBSD's view of QNX's `st`.
pub fn from_qnx(st: &QnxStat) -> Stat {
    Stat {
        st_dev: u64::from(st.st_dev),
        st_ino: st.st_ino,
        st_nlink: u64::from(st.st_nlink),
        st_mode: st.st_mode as u16,
        st_uid: st.st_uid,
        st_gid: st.st_gid,
        st_rdev: u64::from(st.st_rdev),
        st_atim: st.st_atim,
        st_mtim: st.st_mtim,
        st_ctim: st.st_ctim,
        st_birthtim: st.st_ctim,
        st_size: st.st_size,
        st_blocks: st.st_blocks,
        st_blksize: st.st_blksize as i32,
        ..Stat::default()
    }
}

/// QNX's `struct stat`, form `_STAT_FORM_T64_2008`.
pub fn to_qnx(st: &Stat, extra: Extra) -> QnxStat {
    QnxStat {
        st_ino: st.st_ino,
        st_size: st.st_size,
        st_dev: st.st_dev as u32,
        st_rdev: st.st_rdev as u32,
        st_uid: st.st_uid,
        st_gid: st.st_gid,
        old_times: [0; 3],
        st_mode: u32::from(st.st_mode) | extra.mode_bits,
        st_nlink: st.st_nlink as u32,
        st_blocksize: extra.blocksize,
        st_nblocks: st.st_blocks as i32,
        st_blksize: st.st_blksize as u32,
        st_blocks: st.st_blocks,
        st_mtim: st.st_mtim,
        st_atim: st.st_atim,
        st_ctim: st.st_ctim,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_goes_there_and_back() {
        let b = Stat {
            st_ino: 7,
            st_size: 4096,
            st_mode: 0o100644,
            st_nlink: 2,
            st_blocks: 8,
            st_blksize: 512,
            st_mtim: [5, 6],
            ..Stat::default()
        };
        let q = to_qnx(&b, EXTRA);
        assert_eq!(q.st_blocksize, 512);
        let back = from_qnx(&q);
        assert_eq!((back.st_ino, back.st_size, back.st_mode), (7, 4096, 0o100644));
        assert_eq!((back.st_nlink, back.st_blocks, back.st_mtim), (2, 8, [5, 6]));
    }

    #[test]
    fn the_extra_goes_in() {
        let q = to_qnx(&Stat::default(), Extra { blocksize: 1024, mode_bits: 0x10000 });
        assert_eq!((q.st_blocksize, q.st_mode), (1024, 0x10000));
    }
}

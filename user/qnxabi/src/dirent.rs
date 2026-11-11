//! Directory records between FreeBSD's `struct dirent` and QNX's.

const BSD_NAME_AT: usize = 24;
const QNX_NAME_AT: usize = 20;

/// QNX's records for FreeBSD's `bsd`, into `out`: the bytes written. A QNX
/// record is never longer, so `out` as long as `bsd` is enough.
pub fn to_qnx(bsd: &[u8], out: &mut [u8]) -> usize {
    let (mut from, mut to) = (0, 0);
    while from + BSD_NAME_AT <= bsd.len() {
        let u16_at = |at: usize| usize::from(u16::from_le_bytes([bsd[from + at], bsd[from + at + 1]]));
        let (reclen, namelen) = (u16_at(16), u16_at(20));
        if reclen < BSD_NAME_AT || from + reclen > bsd.len() || BSD_NAME_AT + namelen > reclen {
            break;
        }
        let length = (QNX_NAME_AT + namelen + 1).next_multiple_of(8);
        if to + length > out.len() {
            break;
        }
        let record = &mut out[to..to + length];
        record.fill(0);
        record[..16].copy_from_slice(&bsd[from..from + 16]);
        record[16..18].copy_from_slice(&(length as u16).to_le_bytes());
        record[18..20].copy_from_slice(&(namelen as u16).to_le_bytes());
        record[QNX_NAME_AT..QNX_NAME_AT + namelen].copy_from_slice(&bsd[from + BSD_NAME_AT..from + BSD_NAME_AT + namelen]);
        from += reclen;
        to += length;
    }
    to
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ino: u64, off: u64, name: &[u8]) -> [u8; 32] {
        let mut r = [0u8; 32];
        r[0..8].copy_from_slice(&ino.to_le_bytes());
        r[8..16].copy_from_slice(&off.to_le_bytes());
        r[16..18].copy_from_slice(&32u16.to_le_bytes());
        r[20..22].copy_from_slice(&(name.len() as u16).to_le_bytes());
        r[24..24 + name.len()].copy_from_slice(name);
        r
    }

    #[test]
    fn records_go_across() {
        let mut bsd = [0u8; 64];
        bsd[..32].copy_from_slice(&record(5, 1, b"a"));
        bsd[32..].copy_from_slice(&record(6, 2, b"file.txt"));
        let mut out = [0u8; 64];
        let n = to_qnx(&bsd, &mut out);
        assert_eq!(n, 24 + 32);
        assert_eq!(out[0], 5);
        assert_eq!(u16::from_le_bytes([out[16], out[17]]), 24);
        assert_eq!(&out[20..22], b"a\0");
        assert_eq!(out[24], 6);
        assert_eq!(&out[44..52], b"file.txt");
        assert_eq!(out[52], 0, "a NUL");
    }

    #[test]
    fn a_broken_record_ends_it() {
        assert_eq!(to_qnx(&[0u8; 32], &mut [0u8; 32]), 0);
    }
}

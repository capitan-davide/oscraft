//! On-disk structures of a QNX OS 8.0 boot image.
//!
//! Layouts follow the SDP 8.0.5 headers `<sys/startup.h>` and `<sys/image.h>`
//! (little-endian, AArch64). Offsets are given per field; all multi-byte
//! values are little-endian.

use sha2::{Digest, Sha512};

pub const EM_AARCH64: u16 = 183;

// ---- startup header (<sys/startup.h>) ----

pub const STARTUP_HDR_SIGNATURE: u32 = 0x00ff_7eeb;
pub const STARTUP_HDR_VERSION: u16 = 1;
/// `sizeof(struct startup_header)`.
pub const STARTUP_HDR_SIZE: usize = 0x100;
pub const STARTUP_HDR_FLAGS1_VIRTUAL: u8 = 0x01;
pub const STARTUP_HDR_FLAGS1_BIGENDIAN: u8 = 0x02;
pub const STARTUP_HDR_FLAGS1_COMPRESS_MASK: u8 = 0x1c;
pub const STARTUP_HDR_FLAGS1_TRAILER_V2: u8 = 0x20;

/// `struct startup_header`, without the `info[]` array (always zero here).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StartupHeader {
    pub signature: u32,
    pub version: u16,
    pub flags1: u8,
    pub flags2: u8,
    pub header_size: u16,
    pub machine: u16,
    pub startup_vaddr: u32,
    pub paddr_bias: u32,
    pub image_paddr: u32,
    pub ram_paddr: u32,
    pub ram_size: u32,
    pub startup_size: u32,
    pub stored_size: u32,
    pub imagefs_paddr: u32,
    pub imagefs_size: u32,
    pub preboot_size: u16,
    pub addr_off: u64,
}

impl StartupHeader {
    pub fn to_bytes(&self) -> [u8; STARTUP_HDR_SIZE] {
        let mut b = [0u8; STARTUP_HDR_SIZE];
        put_u32(&mut b, 0, self.signature);
        put_u16(&mut b, 4, self.version);
        b[6] = self.flags1;
        b[7] = self.flags2;
        put_u16(&mut b, 8, self.header_size);
        put_u16(&mut b, 10, self.machine);
        put_u32(&mut b, 12, self.startup_vaddr);
        put_u32(&mut b, 16, self.paddr_bias);
        put_u32(&mut b, 20, self.image_paddr);
        put_u32(&mut b, 24, self.ram_paddr);
        put_u32(&mut b, 28, self.ram_size);
        put_u32(&mut b, 32, self.startup_size);
        put_u32(&mut b, 36, self.stored_size);
        put_u32(&mut b, 40, self.imagefs_paddr);
        put_u32(&mut b, 44, self.imagefs_size);
        put_u16(&mut b, 48, self.preboot_size);
        // 50: zero0, 52: zero[1]
        put_u64(&mut b, 56, self.addr_off);
        // 64..256: info[48], filled in by an IPL at boot time
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < STARTUP_HDR_SIZE || get_u32(b, 0) != STARTUP_HDR_SIGNATURE {
            return None;
        }
        Some(Self {
            signature: get_u32(b, 0),
            version: get_u16(b, 4),
            flags1: b[6],
            flags2: b[7],
            header_size: get_u16(b, 8),
            machine: get_u16(b, 10),
            startup_vaddr: get_u32(b, 12),
            paddr_bias: get_u32(b, 16),
            image_paddr: get_u32(b, 20),
            ram_paddr: get_u32(b, 24),
            ram_size: get_u32(b, 28),
            startup_size: get_u32(b, 32),
            stored_size: get_u32(b, 36),
            imagefs_paddr: get_u32(b, 40),
            imagefs_size: get_u32(b, 44),
            preboot_size: get_u16(b, 48),
            addr_off: get_u64(b, 56),
        })
    }
}

// ---- boot arguments (<sys/startup.h> struct bootargs_entry) ----

/// Placeholder that marks the boot-argument area in a startup program.
pub const BOOTARGS_SIGNATURE: &[u8; 8] = b"ddpvbskr";
/// `shdr_addr` value meaning "64-bit address appended after the strings".
pub const SHDR_ADDR_64: u32 = 0xffff_ffff;

/// Encode a `struct bootargs_entry`: size, argc, envc, startup header
/// address, then the argument and environment strings, NUL-terminated.
pub fn encode_bootargs(argv: &[String], envv: &[String], shdr_addr: u64) -> anyhow::Result<Vec<u8>> {
    // `struct bootargs_entry`: a 16-bit size and `_Int8t` counts (review
    // TOOLS-6: they wrapped).
    anyhow::ensure!(argv.len() <= i8::MAX as usize && envv.len() <= i8::MAX as usize,
        "{} arguments and {} environment strings: at most {} each", argv.len(), envv.len(), i8::MAX);
    let mut b = vec![0u8; 8];
    for s in argv.iter().chain(envv) {
        b.extend_from_slice(s.as_bytes());
        b.push(0);
    }
    let shdr32 = match u32::try_from(shdr_addr) {
        Ok(a) if a != SHDR_ADDR_64 => a,
        _ => {
            b.extend_from_slice(&shdr_addr.to_le_bytes());
            SHDR_ADDR_64
        }
    };
    let size = b.len();
    anyhow::ensure!(size <= u16::MAX as usize, "boot arguments of {size} bytes: at most {}", u16::MAX);
    b[0] = size as u8;
    b[1] = (size >> 8) as u8;
    b[2] = argv.len() as u8;
    b[3] = envv.len() as u8;
    put_u32(&mut b, 4, shdr32);
    Ok(b)
}

// ---- bootstrap executables (<sys/image.h>) ----

/// `struct ifs_bootstrap_head.signature`: 'BSHD'.
pub const IFS_BOOTSTRAP_SIGNATURE: u32 = 0x4253_4844;
/// `sizeof(struct ifs_bootstrap_head)`; it sits immediately before the entry point.
pub const IFS_BOOTSTRAP_HEAD_SIZE: u64 = 16;
/// Offset of `args` in `struct ifs_bootstrap_data`.
pub const IFS_BOOTSTRAP_DATA_ARGS: u64 = 16;

pub const IFS_INO_PROCESSED_ELF: u32 = 0x8000_0000;
pub const IFS_INO_RUNONCE_ELF: u32 = 0x4000_0000;
pub const IFS_INO_BOOTSTRAP_EXE: u32 = 0x2000_0000;
pub const IFS_INO_FLAGS: u32 = 0xe000_0000;

// ---- image filesystem (<sys/image.h>) ----

pub const IMAGE_SIGNATURE: &[u8; 7] = b"imagefs";
pub const IMAGE_FLAGS_BIGENDIAN: u8 = 0x01;
pub const IMAGE_FLAGS_READONLY: u8 = 0x02;
pub const IMAGE_FLAGS_INO_BITS: u8 = 0x04;
pub const IMAGE_FLAGS_SORTED: u8 = 0x08;
pub const IMAGE_FLAGS_TRAILER_V2: u8 = 0x10;
/// `sizeof(struct image_header)` up to (not including) `mountpoint`.
pub const IMAGE_HDR_SIZE: usize = 88;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImageHeader {
    pub flags: u8,
    pub image_size: u32,
    pub hdr_dir_size: u32,
    pub dir_offset: u32,
    pub boot_ino: [u32; 4],
    pub script_ino: u32,
    pub chain_paddr: u32,
    pub mountflags: u32,
    pub mountpoint: String,
}

impl ImageHeader {
    /// Header plus NUL-terminated mount point, padded to 4 bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = vec![0u8; IMAGE_HDR_SIZE];
        b[..7].copy_from_slice(IMAGE_SIGNATURE);
        b[7] = self.flags;
        put_u32(&mut b, 8, self.image_size);
        put_u32(&mut b, 12, self.hdr_dir_size);
        put_u32(&mut b, 16, self.dir_offset);
        for (i, ino) in self.boot_ino.iter().enumerate() {
            put_u32(&mut b, 20 + 4 * i, *ino);
        }
        put_u32(&mut b, 36, self.script_ino);
        put_u32(&mut b, 40, self.chain_paddr);
        // 44..84: spare[10]
        put_u32(&mut b, 84, self.mountflags);
        b.extend_from_slice(self.mountpoint.as_bytes());
        b.push(0);
        pad_to(&mut b, 4);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        if b.len() < IMAGE_HDR_SIZE || &b[..7] != IMAGE_SIGNATURE {
            return None;
        }
        Some(Self {
            flags: b[7],
            image_size: get_u32(b, 8),
            hdr_dir_size: get_u32(b, 12),
            dir_offset: get_u32(b, 16),
            boot_ino: [
                get_u32(b, 20),
                get_u32(b, 24),
                get_u32(b, 28),
                get_u32(b, 32),
            ],
            script_ino: get_u32(b, 36),
            chain_paddr: get_u32(b, 40),
            mountflags: get_u32(b, 84),
            mountpoint: get_cstr(b, IMAGE_HDR_SIZE),
        })
    }
}

pub const S_IFMT: u32 = 0o170000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFLNK: u32 = 0o120000;

/// Size of `struct image_attr`, the common prefix of every dirent.
pub const DIRENT_ATTR_SIZE: usize = 24;

/// One `union image_dirent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dirent {
    pub ino: u32,
    pub mode: u32,
    pub gid: u32,
    pub uid: u32,
    pub mtime: u32,
    pub path: String,
    pub kind: DirentKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirentKind {
    File { offset: u32, size: u32 },
    Dir,
    Symlink { target: String },
}

impl Dirent {
    /// Encoded size, padded to 4 bytes.
    pub fn encoded_len(&self) -> usize {
        let path = self.path.len() + 1;
        let len = match &self.kind {
            DirentKind::File { .. } => DIRENT_ATTR_SIZE + 8 + path,
            DirentKind::Dir => DIRENT_ATTR_SIZE + path,
            DirentKind::Symlink { target } => DIRENT_ATTR_SIZE + 4 + path + target.len() + 1,
        };
        len.next_multiple_of(4)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let size = self.encoded_len();
        let mut b = vec![0u8; DIRENT_ATTR_SIZE];
        put_u16(&mut b, 0, size as u16);
        // 2: extattr_offset (none)
        put_u32(&mut b, 4, self.ino);
        put_u32(&mut b, 8, self.mode);
        put_u32(&mut b, 12, self.gid);
        put_u32(&mut b, 16, self.uid);
        put_u32(&mut b, 20, self.mtime);
        match &self.kind {
            DirentKind::File { offset, size } => {
                b.extend_from_slice(&offset.to_le_bytes());
                b.extend_from_slice(&size.to_le_bytes());
                push_cstr(&mut b, &self.path);
            }
            DirentKind::Dir => push_cstr(&mut b, &self.path),
            DirentKind::Symlink { target } => {
                // sym_offset is relative to the start of the path.
                let sym_offset = self.path.len() + 1;
                b.extend_from_slice(&(sym_offset as u16).to_le_bytes());
                b.extend_from_slice(&(target.len() as u16).to_le_bytes());
                push_cstr(&mut b, &self.path);
                push_cstr(&mut b, target);
            }
        }
        b.resize(size, 0);
        b
    }

    /// Decode one dirent; `None` at the zero-size terminator, or if `b`
    /// doesn't hold the whole entry (`Image::parse` tells them apart).
    pub fn from_bytes(b: &[u8]) -> Option<(Self, usize)> {
        let size = get_u16(b.get(..2)?, 0) as usize;
        if size == 0 || size < DIRENT_ATTR_SIZE + 4 || size > b.len() {
            return None;
        }
        let b = &b[..size];
        let mode = get_u32(b, 8);
        let (path, kind) = match mode & S_IFMT {
            S_IFREG if b.len() < DIRENT_ATTR_SIZE + 8 => return None,
            S_IFREG => (
                get_cstr(b, 32),
                DirentKind::File {
                    offset: get_u32(b, 24),
                    size: get_u32(b, 28),
                },
            ),
            S_IFLNK => {
                let sym_offset = get_u16(b, 24) as usize;
                let sym_size = get_u16(b, 26) as usize;
                let start = 28 + sym_offset;
                let target = String::from_utf8_lossy(b.get(start..start + sym_size)?).into_owned();
                (get_cstr(b, 28), DirentKind::Symlink { target })
            }
            // Directories, and devices (not produced by this mkifs) as directories.
            _ => (get_cstr(b, DIRENT_ATTR_SIZE), DirentKind::Dir),
        };
        let d = Self {
            ino: get_u32(b, 4),
            mode,
            gid: get_u32(b, 12),
            uid: get_u32(b, 16),
            mtime: get_u32(b, 20),
            path,
            kind,
        };
        Some((d, size))
    }
}

// ---- boot script (<sys/image.h> union script_cmd) ----

pub const SCRIPT_FLAGS_EXTSCHED: u8 = 0x01;
pub const SCRIPT_FLAGS_SESSION: u8 = 0x02;
pub const SCRIPT_FLAGS_SCHED_SET: u8 = 0x04;
pub const SCRIPT_FLAGS_CPU_SET: u8 = 0x08;
pub const SCRIPT_FLAGS_BACKGROUND: u8 = 0x20;
pub const SCRIPT_FLAGS_KDEBUG: u8 = 0x40;
pub const SCRIPT_FLAGS_CRITICAL: u8 = 0x80;

pub const SCRIPT_POLICY_NOCHANGE: u8 = 0;
pub const SCRIPT_POLICY_FIFO: u8 = 1;
pub const SCRIPT_POLICY_RR: u8 = 2;
pub const SCRIPT_POLICY_OTHER: u8 = 3;

pub const SCRIPT_TYPE_EXTERNAL: u8 = 0;
pub const SCRIPT_TYPE_WAITFOR: u8 = 1;
pub const SCRIPT_TYPE_REOPEN: u8 = 2;
pub const SCRIPT_TYPE_DISPLAY_MSG: u8 = 3;
pub const SCRIPT_TYPE_PROCMGR_SYMLINK: u8 = 4;
pub const SCRIPT_TYPE_EXTSCHED_APS: u8 = 5;

/// Wait granularity of `waitfor`/`reopen`, in milliseconds.
pub const SCRIPT_CHECKS_MS: u32 = 100;

// ---- trailers and checksums ----

/// `sizeof(struct startup_trailer_v2)` and `sizeof(struct image_trailer_v2)`.
pub const TRAILER_V2_SIZE: usize = 68;

/// Sum of the little-endian 32-bit words of `data` (length a multiple of 4).
pub fn sum32(data: &[u8]) -> u32 {
    debug_assert!(data.len().is_multiple_of(4));
    data.chunks_exact(4).fold(0u32, |s, w| {
        s.wrapping_add(u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
    })
}

/// Build a v2 trailer for `region` (header up to the trailer): SHA-512 of the
/// region, then a checksum that makes the word sum of region + trailer zero.
pub fn trailer_v2(region: &[u8]) -> [u8; TRAILER_V2_SIZE] {
    let mut t = [0u8; TRAILER_V2_SIZE];
    t[..64].copy_from_slice(&Sha512::digest(region));
    let sum = sum32(region).wrapping_add(sum32(&t[..64]));
    put_u32(&mut t, 64, 0u32.wrapping_sub(sum));
    t
}

/// Check a v2 trailer at the end of `region_with_trailer`.
pub fn check_trailer_v2(region_with_trailer: &[u8]) -> (bool, bool) {
    let n = region_with_trailer.len();
    if n < TRAILER_V2_SIZE || !n.is_multiple_of(4) {
        return (false, false);
    }
    let (region, trailer) = region_with_trailer.split_at(n - TRAILER_V2_SIZE);
    let sha_ok = Sha512::digest(region).as_slice() == &trailer[..64];
    let sum_ok = sum32(region_with_trailer) == 0;
    (sha_ok, sum_ok)
}

// ---- helpers ----

pub fn put_u16(b: &mut [u8], off: usize, v: u16) {
    b[off..off + 2].copy_from_slice(&v.to_le_bytes());
}
pub fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}
pub fn put_u64(b: &mut [u8], off: usize, v: u64) {
    b[off..off + 8].copy_from_slice(&v.to_le_bytes());
}
pub fn get_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}
pub fn get_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
pub fn get_u64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}
pub fn get_cstr(b: &[u8], off: usize) -> String {
    let tail = b.get(off..).unwrap_or_default();
    let end = tail.iter().position(|&c| c == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).into_owned()
}
pub fn push_cstr(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(s.as_bytes());
    b.push(0);
}
pub fn pad_to(b: &mut Vec<u8>, align: usize) {
    b.resize(b.len().next_multiple_of(align), 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_header_round_trip() {
        let h = StartupHeader {
            signature: STARTUP_HDR_SIGNATURE,
            version: 1,
            flags1: 0x21,
            header_size: 0x100,
            machine: EM_AARCH64,
            startup_vaddr: 0x81800,
            image_paddr: 0x80fa0,
            startup_size: 0x29148,
            preboot_size: 0xfa0,
            ..Default::default()
        };
        assert_eq!(StartupHeader::from_bytes(&h.to_bytes()), Some(h));
    }

    #[test]
    fn trailer_makes_word_sum_zero() {
        let region: Vec<u8> = (0..400u32).flat_map(|i| (i * 7919).to_le_bytes()).collect();
        let mut all = region.clone();
        all.extend_from_slice(&trailer_v2(&region));
        assert_eq!(check_trailer_v2(&all), (true, true));
    }

    #[test]
    fn dirent_sizes_match_reference_image() {
        // Sizes observed in the QNX 8.0 RPi4 BSP image.
        let file = |p: &str| Dirent {
            ino: 0,
            mode: S_IFREG,
            gid: 0,
            uid: 0,
            mtime: 0,
            path: p.into(),
            kind: DirentKind::File { offset: 0, size: 0 },
        };
        let dir = |p: &str| Dirent {
            kind: DirentKind::Dir,
            mode: S_IFDIR,
            ..file(p)
        };
        let link = |p: &str, t: &str| Dirent {
            kind: DirentKind::Symlink { target: t.into() },
            mode: S_IFLNK,
            ..file(p)
        };
        assert_eq!(dir("").encoded_len(), 28);
        assert_eq!(dir("bin").encoded_len(), 28);
        assert_eq!(file("bin/confstr").encoded_len(), 44);
        assert_eq!(file("proc/boot/procnto-smp-instr").encoded_len(), 60);
        assert_eq!(link("bin/cat", "/usr/bin/toybox").encoded_len(), 52);
        assert_eq!(link("lib/libc.so", "libc.so.6").encoded_len(), 52);
    }

    #[test]
    fn dirent_round_trip() {
        let d = Dirent {
            ino: 0x133,
            mode: S_IFLNK | 0o777,
            gid: 0,
            uid: 0,
            mtime: 5,
            path: "bin/cat".into(),
            kind: DirentKind::Symlink {
                target: "/usr/bin/toybox".into(),
            },
        };
        let b = d.to_bytes();
        assert_eq!(Dirent::from_bytes(&b), Some((d, 52)));
    }

    #[test]
    fn bootargs_limits() {
        let many: Vec<String> = (0..128).map(|i| i.to_string()).collect();
        assert!(encode_bootargs(&many[..127], &[], 0x80fa0).is_ok());
        assert!(encode_bootargs(&many, &[], 0x80fa0).is_err());
        assert!(encode_bootargs(&["x".repeat(70_000)], &[], 0x80fa0).is_err());
    }

    #[test]
    fn bootargs_layout() {
        let b = encode_bootargs(&["prog".into(), "-v".into()], &["A=1".into()], 0x80fa0).unwrap();
        assert_eq!(b.len(), 8 + 5 + 3 + 4);
        assert_eq!((b[0], b[1], b[2], b[3]), (20, 0, 2, 1));
        assert_eq!(get_u32(&b, 4), 0x80fa0);
        assert_eq!(&b[8..], b"prog\0-v\0A=1\0");
    }
}

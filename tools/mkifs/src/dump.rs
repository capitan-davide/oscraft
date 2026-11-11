//! Reading and describing boot images (`dumpifs`).

use std::fmt::Write;

use anyhow::{Context, Result, bail};

use crate::elf::Elf;
use crate::format::*;
use crate::script;

/// A parsed boot image.
pub struct Image<'a> {
    pub data: &'a [u8],
    /// File offset and contents of the startup header, if bootable.
    pub startup: Option<(usize, StartupHeader)>,
    /// File offset and header of the image filesystem.
    pub ifs_offset: usize,
    pub ifs: ImageHeader,
    pub dirents: Vec<Dirent>,
}

impl<'a> Image<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let startup = (0..data.len().saturating_sub(STARTUP_HDR_SIZE))
            .step_by(4)
            .filter(|&o| get_u32(data, o) == STARTUP_HDR_SIGNATURE)
            .find_map(|o| {
                StartupHeader::from_bytes(&data[o..])
                    .filter(|h| {
                        h.version == STARTUP_HDR_VERSION
                            && h.header_size as usize == STARTUP_HDR_SIZE
                    })
                    .map(|h| (o, h))
            });
        let ifs_offset = match &startup {
            Some((o, h)) => o + h.startup_size as usize,
            None => find(data, IMAGE_SIGNATURE).context("no image filesystem found")?,
        };
        let ifs = ImageHeader::from_bytes(data.get(ifs_offset..).unwrap_or_default())
            .context("bad image filesystem header")?;
        if ifs.flags & IMAGE_FLAGS_BIGENDIAN != 0 {
            bail!("big-endian images are not supported");
        }
        // Each entry is checked against the data: a truncated or corrupt
        // image (a partial TFTP transfer) is an error, not a panic (review
        // TOOLS-5). Entries with inode 0 are removed ones, skipped.
        let mut dirents = Vec::new();
        let mut p = ifs_offset + ifs.dir_offset as usize;
        loop {
            let size = data
                .get(p..p + 2)
                .map(|s| get_u16(s, 0) as usize)
                .with_context(|| format!("image truncated in its directory, at {p:#x}"))?;
            if size == 0 {
                break;
            }
            let (d, size) = data
                .get(p..)
                .and_then(Dirent::from_bytes)
                .with_context(|| format!("truncated or corrupt directory entry at {p:#x}"))?;
            if d.ino != 0 {
                dirents.push(d);
            }
            p += size;
        }
        Ok(Self {
            data,
            startup,
            ifs_offset,
            ifs,
            dirents,
        })
    }

    pub fn file(&self, d: &Dirent) -> Option<&'a [u8]> {
        let DirentKind::File { offset, size } = d.kind else {
            return None;
        };
        let start = self.ifs_offset + offset as usize;
        self.data.get(start..start + size as usize)
    }

    /// Human-readable description; `verbose` adds the script and bootstrap details.
    pub fn describe(&self, verbose: bool) -> String {
        let mut s = String::new();
        if let Some((o, h)) = &self.startup {
            let _ = writeln!(s, "startup header at file offset {o:#x}");
            let _ = writeln!(
                s,
                "  flags1={:#04x} machine={} startup_vaddr={:#x} image_paddr={:#x} ram_paddr={:#x}",
                h.flags1, h.machine, h.startup_vaddr, h.image_paddr, h.ram_paddr
            );
            let _ = writeln!(
                s,
                "  startup_size={:#x} stored_size={:#x} ram_size={:#x} imagefs_size={:#x} preboot_size={:#x}",
                h.startup_size, h.stored_size, h.ram_size, h.imagefs_size, h.preboot_size
            );
            if h.flags1 & STARTUP_HDR_FLAGS1_COMPRESS_MASK != 0 {
                let _ = writeln!(s, "  compressed image filesystem (not decoded)");
            }
            let region = &self.data[*o..o + h.startup_size as usize];
            let _ = writeln!(
                s,
                "  trailer: {}",
                check(region, h.flags1 & STARTUP_HDR_FLAGS1_TRAILER_V2 != 0)
            );
        }
        let i = &self.ifs;
        let _ = writeln!(s, "image filesystem at file offset {:#x}", self.ifs_offset);
        let _ = writeln!(
            s,
            "  flags={:#04x} image_size={:#x} hdr_dir_size={:#x} dir_offset={:#x} mountpoint={:?}",
            i.flags, i.image_size, i.hdr_dir_size, i.dir_offset, i.mountpoint
        );
        let _ = writeln!(
            s,
            "  boot_ino={:x?} script_ino={:#x}",
            i.boot_ino, i.script_ino
        );
        if let Some(region) = self
            .data
            .get(self.ifs_offset..self.ifs_offset + i.image_size as usize)
        {
            let _ = writeln!(
                s,
                "  trailer: {}",
                check(region, i.flags & IMAGE_FLAGS_TRAILER_V2 != 0)
            );
        }
        let _ = writeln!(
            s,
            "{:>10} {:>10} {:>8} {:>6} path",
            "ino", "offset", "size", "mode"
        );
        for d in &self.dirents {
            let (off, size) = match d.kind {
                DirentKind::File { offset, size } => (format!("{offset:#x}"), size.to_string()),
                _ => (String::new(), String::new()),
            };
            let extra = match &d.kind {
                DirentKind::Symlink { target } => format!(" -> {target}"),
                _ => String::new(),
            };
            let _ = writeln!(
                s,
                "{:>#10x} {off:>10} {size:>8} {:>6o} /{}{extra}",
                d.ino, d.mode, d.path
            );
        }
        if verbose {
            for d in &self.dirents {
                if d.ino & IFS_INO_BOOTSTRAP_EXE != 0 {
                    let _ = writeln!(s, "bootstrap /{}:", d.path);
                    s.push_str(&self.file(d).map(describe_bootstrap).unwrap_or_default());
                }
                if d.ino == i.script_ino && i.script_ino != 0 {
                    let _ = writeln!(s, "script /{}:", d.path);
                    for line in self.file(d).map(script::decode).unwrap_or_default() {
                        let _ = writeln!(s, "  {line}");
                    }
                }
            }
        }
        s
    }
}

fn check(region: &[u8], v2: bool) -> String {
    if v2 {
        let (sha, sum) = check_trailer_v2(region);
        format!("sha512 {}, checksum {}", ok(sha), ok(sum))
    } else {
        format!(
            "checksum {}",
            ok(region.len().is_multiple_of(4) && sum32(region) == 0)
        )
    }
}

fn ok(b: bool) -> &'static str {
    if b { "ok" } else { "BAD" }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Segments and boot arguments of a processed bootstrap executable.
fn describe_bootstrap(file: &[u8]) -> String {
    let mut s = String::new();
    let Ok(elf) = Elf::parse(file.to_vec()) else {
        return "  not an ELF file\n".into();
    };
    let _ = writeln!(s, "  entry {:#x}", elf.e_entry);
    for p in elf.loads() {
        let _ = writeln!(
            s,
            "  LOAD vaddr={:#x} paddr={:#x} offset={:#x} filesz={:#x} memsz={:#x} flags={}",
            p.p_vaddr, p.p_paddr, p.p_offset, p.p_filesz, p.p_memsz, p.p_flags
        );
    }
    let args = elf
        .read(elf.e_entry - IFS_BOOTSTRAP_HEAD_SIZE, 16)
        .filter(|h| get_u32(h, 0) == IFS_BOOTSTRAP_SIGNATURE)
        .and_then(|h| elf.read(get_u64(h, 8) + IFS_BOOTSTRAP_DATA_ARGS, 8))
        .map(|a| get_u64(a, 0))
        .and_then(|a| elf.read(a, 8).map(|h| (a, h)))
        .and_then(|(a, h)| {
            elf.read(a, (h[0] as usize) | (h[1] as usize) << 8)
                .map(|b| (b, h[2], h[3]))
        });
    match args {
        Some((b, argc, envc)) => {
            let strings: Vec<_> = b
                .get(8..)
                .unwrap_or_default()
                .split(|&c| c == 0)
                .map(String::from_utf8_lossy)
                .collect();
            let argc = argc as usize;
            let _ = writeln!(
                s,
                "  argv: {}",
                strings[..argc.min(strings.len())].join(" ")
            );
            let env = strings.get(argc..argc + envc as usize).unwrap_or_default();
            let _ = writeln!(s, "  env:  {}", env.join(" "));
            if let Some(h) = b.get(4..8) {
                let _ = writeln!(s, "  startup header: {:#x}", get_u32(h, 0));
            }
        }
        None => s.push_str("  no boot arguments found\n"),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image filesystem of two files, then that image cut at every
    /// length: parsing and describing never panic (review TOOLS-5).
    #[test]
    fn truncated_images_are_errors_not_panics() {
        let file = |ino: u32, path: &str| Dirent {
            ino,
            mode: S_IFREG | 0o644,
            gid: 0,
            uid: 0,
            mtime: 0,
            path: path.into(),
            kind: DirentKind::File { offset: 0, size: 4 },
        };
        let mut hdr = ImageHeader {
            flags: 0,
            image_size: 0,
            hdr_dir_size: 0,
            dir_offset: 0,
            boot_ino: [0; 4],
            script_ino: 0,
            chain_paddr: 0,
            mountflags: 0,
            mountpoint: "/".into(),
        };
        hdr.dir_offset = hdr.to_bytes().len() as u32;
        let mut img = hdr.to_bytes();
        img.extend(file(2, "a").to_bytes());
        img.extend(file(0, "removed").to_bytes());
        img.extend(file(3, "b").to_bytes());
        img.extend([0u8; 4]); // the terminator
        let whole = Image::parse(&img).unwrap();
        assert_eq!(whole.dirents.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        for n in 0..img.len() {
            if let Ok(i) = Image::parse(&img[..n]) {
                let _ = i.describe(true);
            }
        }
    }
}

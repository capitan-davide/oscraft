//! The image filesystem (QNX `pathmgr/imagefs.c`): read-only access to the
//! files packed into the boot image (format: `<sys/image.h>`, see
//! the book's boot chapter). Startup tells us where it is through
//! the system page's `imagefs` memory region.

use alloc::string::String;
use alloc::vec::Vec;

use crate::arch::mmu::Prot;
use crate::sync::InitOnce;
use crate::syspage::Syspage;
use crate::vm::kmem;

const IMAGE_SIGNATURE: &[u8; 7] = b"imagefs";
pub const S_IFMT: u32 = 0o170000;
pub const S_IFREG: u32 = 0o100000;

pub struct ImageFs {
    data: &'static [u8],
    dir_offset: usize,
    script_ino: u32,
    /// The entries by path (offset, inode, mode), and the directories
    /// implied by the paths of their contents: built once, for lookups
    /// on every open().
    index: alloc::collections::BTreeMap<&'static [u8], (usize, u32, u32)>,
    implied: alloc::collections::BTreeSet<&'static [u8]>,
}

#[derive(Debug, Clone, Copy)]
pub struct File {
    pub data: &'static [u8],
}

pub const S_IFDIR: u32 = 0o040000;
pub const S_IFLNK: u32 = 0o120000;

/// A directory entry (`union image_dirent`): attributes and contents.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub ino: u32,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub mtime: u32,
    /// File contents, symlink target, or empty for a directory.
    pub data: &'static [u8],
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.mode & S_IFMT == S_IFDIR
    }
}

static FS: InitOnce<ImageFs> = InitOnce::new();

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn cstr(b: &[u8], o: usize) -> &[u8] {
    let end = b[o..]
        .iter()
        .position(|&c| c == 0)
        .map_or(b.len(), |n| o + n);
    &b[o..end]
}

/// The image filesystem's physical address.
static PHYS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// The physical address of `bytes`, if they lie in the image filesystem
/// (a file's data, for mapping it in place).
pub fn phys_of(bytes: &[u8]) -> Option<u64> {
    let fs = get()?;
    let (base, at) = (fs.data.as_ptr() as u64, bytes.as_ptr() as u64);
    (at >= base && at + bytes.len() as u64 <= base + fs.data.len() as u64)
        .then(|| PHYS.load(core::sync::atomic::Ordering::Relaxed) + (at - base))
}

/// Map the image filesystem read-only and check its header.
pub fn init(sp: &Syspage) -> Option<&'static ImageFs> {
    let region = sp.asinfo().find(|a| sp.string(a.name) == "imagefs")?;
    let len = region.end - region.start + 1;
    let va = kmem::map_phys(region.start, len, Prot::ReadOnly)?;
    // SAFETY: just mapped, read-only, for the kernel's lifetime.
    let data: &'static [u8] = unsafe { core::slice::from_raw_parts(va as *const u8, len as usize) };
    if &data[..7] != IMAGE_SIGNATURE {
        return None;
    }
    let size = (u32_at(data, 8) as usize).min(data.len());
    let mut fs = ImageFs {
        data: &data[..size],
        dir_offset: u32_at(data, 16) as usize,
        script_ino: u32_at(data, 36),
        index: alloc::collections::BTreeMap::new(),
        implied: alloc::collections::BTreeSet::new(),
    };
    let entries: Vec<_> = fs.dirents().collect();
    for (off, ino, mode, path) in entries {
        // The first entry of a path counts, as a scan would find it.
        fs.index.entry(path).or_insert((off, ino, mode));
        for (i, _) in path.iter().enumerate().filter(|&(_, &c)| c == b'/') {
            fs.implied.insert(&path[..i]);
        }
    }
    FS.set(fs).ok()?;
    PHYS.store(region.start, core::sync::atomic::Ordering::Relaxed);
    // Its programs' code runs in place (`map::map_image`): whatever the
    // loader wrote into the data cache must reach instruction fetches.
    crate::arch::mmu::sync_icache(va, len);
    FS.get()
}

pub fn get() -> Option<&'static ImageFs> {
    FS.get()
}

impl ImageFs {
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// (dirent offset, ino, mode, path) for each entry.
    fn dirents(&self) -> impl Iterator<Item = (usize, u32, u32, &'static [u8])> + '_ {
        let data = self.data;
        let mut off = self.dir_offset;
        core::iter::from_fn(move || {
            let size = u16_at(data, off) as usize;
            if size == 0 {
                return None;
            }
            let (ino, mode) = (u32_at(data, off + 4), u32_at(data, off + 8));
            let path_at = match mode & S_IFMT {
                S_IFREG => 32,
                S_IFLNK => 28,
                _ => 24,
            };
            let item = (off, ino, mode, cstr(data, off + path_at));
            off += size;
            Some(item)
        })
    }

    pub fn entries(&self) -> usize {
        self.dirents().count()
    }

    fn file_at(&self, off: usize, mode: u32) -> Option<File> {
        if mode & S_IFMT != S_IFREG {
            return None;
        }
        let (start, size) = (
            u32_at(self.data, off + 24) as usize,
            u32_at(self.data, off + 28) as usize,
        );
        Some(File {
            data: self.data.get(start..start + size)?,
        })
    }

    fn entry_at(&self, off: usize, ino: u32, mode: u32) -> Option<Entry> {
        let d = self.data;
        let data: &'static [u8] = match mode & S_IFMT {
            S_IFREG => self.file_at(off, mode)?.data,
            S_IFLNK => {
                let (o, n) = (u16_at(d, off + 24) as usize, u16_at(d, off + 26) as usize);
                // sym_offset counts from the path (offset 28).
                d.get(off + 28 + o..off + 28 + o + n)?
            }
            _ => &[],
        };
        Some(Entry {
            ino,
            mode,
            gid: u32_at(d, off + 12),
            uid: u32_at(d, off + 16),
            mtime: u32_at(d, off + 20),
            data,
        })
    }

    /// Any entry by path, without following symbolic links. The image
    /// root ("" or "/") is a directory, also when the image does not list it.
    pub fn entry(&self, path: &str) -> Option<Entry> {
        let path = path.trim_matches('/').as_bytes();
        if let Some(&(off, ino, mode)) = self.index.get(path) {
            return self.entry_at(off, ino, mode);
        }
        // Directories implied by the paths of their contents.
        let implied = path.is_empty() || self.implied.contains(path);
        implied.then_some(Entry {
            ino: 1,
            mode: S_IFDIR | 0o555,
            uid: 0,
            gid: 0,
            mtime: 0,
            data: &[],
        })
    }

    /// The names in directory `dir` (image path, without leading '/') with
    /// their inode numbers, including directories that are only implied by
    /// the paths below them.
    pub fn children(&self, dir: &str) -> Vec<(u32, &'static str)> {
        let dir = dir.trim_matches('/');
        let mut out: Vec<(u32, &'static str)> = Vec::new();
        for (_, ino, _, path) in self.dirents() {
            let Ok(path) = core::str::from_utf8(path) else {
                continue;
            };
            let rest = if dir.is_empty() {
                path
            } else {
                match path.strip_prefix(dir).and_then(|r| r.strip_prefix('/')) {
                    Some(r) => r,
                    None => continue,
                }
            };
            let (name, direct) = match rest.split_once('/') {
                Some((n, _)) => (n, false),
                None => (rest, true),
            };
            if name.is_empty() {
                continue;
            }
            match out.iter_mut().find(|(_, n)| *n == name) {
                Some(e) if direct => e.0 = ino,
                Some(_) => {}
                // An implied directory gets a made-up inode number.
                None => out.push((
                    if direct {
                        ino
                    } else {
                        0x8000_0000 | out.len() as u32
                    },
                    name,
                )),
            }
        }
        out
    }

    /// The entry at `path` (absolute, with or without the leading '/'),
    /// following symbolic links, and its final path (without leading '/').
    pub fn follow(&self, path: &str) -> Option<(Entry, String)> {
        let mut path: String = path.trim_matches('/').into();
        // A bound on the links followed (loops).
        for _ in 0..16 {
            let mut redirected = None;
            let ends = path
                .match_indices('/')
                .map(|(i, _)| i)
                .chain(core::iter::once(path.len()));
            for end in ends {
                let e = self.entry(&path[..end])?;
                if e.mode & S_IFMT == S_IFLNK {
                    let target = core::str::from_utf8(e.data).ok()?;
                    let base = if target.starts_with('/') {
                        ""
                    } else {
                        path[..end].rsplit_once('/').map_or("", |(dir, _)| dir)
                    };
                    redirected = Some(alloc::format!("{base}/{target}{}", &path[end..]));
                    break;
                }
            }
            match redirected {
                Some(p) => path = super::pathmgr::absolute(0, &p),
                None => return Some((self.entry(&path)?, path)),
            }
        }
        None
    }

    /// `path` with the image's symbolic links followed, as far as the image
    /// has its components (without the leading '/'): `/tmp/x` becomes
    /// `dev/shmem/x` where the image links `/tmp` there.
    pub fn resolve(&self, path: &str) -> String {
        let mut path: String = path.trim_matches('/').into();
        for _ in 0..16 {
            let mut redirected = None;
            let ends = path
                .match_indices('/')
                .map(|(i, _)| i)
                .chain(core::iter::once(path.len()));
            for end in ends {
                let Some(e) = self.entry(&path[..end]) else {
                    break;
                };
                if e.mode & S_IFMT == S_IFLNK {
                    let Ok(target) = core::str::from_utf8(e.data) else {
                        return path;
                    };
                    let base = if target.starts_with('/') {
                        ""
                    } else {
                        path[..end].rsplit_once('/').map_or("", |(dir, _)| dir)
                    };
                    redirected = Some(alloc::format!("{base}/{target}{}", &path[end..]));
                    break;
                }
            }
            match redirected {
                Some(p) => path = super::pathmgr::absolute(0, &p),
                None => return path,
            }
        }
        path
    }

    /// A regular file by path (with or without the leading '/').
    pub fn lookup(&self, path: &str) -> Option<File> {
        let path = path.trim_start_matches('/').as_bytes();
        let (off, _, mode, _) = self.dirents().find(|e| e.3 == path)?;
        self.file_at(off, mode)
    }

    /// The compiled boot script, if the image has one.
    pub fn script(&self) -> Option<File> {
        if self.script_ino == 0 {
            return None;
        }
        let (off, _, mode, _) = self.dirents().find(|e| e.1 == self.script_ino)?;
        self.file_at(off, mode)
    }
}

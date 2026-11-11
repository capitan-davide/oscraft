//! Directories and files, as `msdosfs_lookup.c` and `msdosfs_vnops.c`.

use bufcache::BlockDevice;

use crate::fat::FREE;
use crate::name::{checksum, exact_short, invalid, numbered_short, same, short_text};
use crate::{DosTime, Error, Kind, Result, Volume};

pub const ATTR_READONLY: u8 = 0x01;
pub const ATTR_DIR: u8 = 0x10;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LONG: u8 = 0x0f;
const DELETED: u8 = 0xe5;
const SLOT: u32 = 32;

/// A directory: the fixed root of FAT12 and FAT16, or a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dir {
    FixedRoot,
    Chain(u32),
}

/// Where a slot is on the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    sector: u32,
    offset: u32,
}

/// A file or directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub attr: u8,
    pub first_cluster: u32,
    pub size: u32,
    pub mtime: DosTime,
    pub(crate) slot: Option<Slot>,
    /// Its long name's slots, before `slot`.
    pub(crate) long: Vec<Slot>,
    /// Its short name, as on the disk.
    pub(crate) short: [u8; 11],
    /// The cluster of the directory it is in, 0 for a fixed root.
    pub(crate) parent: u32,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attr & ATTR_DIR != 0
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

impl<D: BlockDevice> Volume<D> {
    fn root(&self) -> Dir {
        if self.kind == Kind::Fat32 { Dir::Chain(self.root_cluster) } else { Dir::FixedRoot }
    }

    /// The root directory, as an entry.
    pub fn root_entry(&self) -> Entry {
        let first_cluster = if self.kind == Kind::Fat32 { self.root_cluster } else { 0 };
        Entry {
            name: "/".into(),
            attr: ATTR_DIR,
            first_cluster,
            size: 0,
            mtime: DosTime::default(),
            slot: None,
            long: Vec::new(),
            short: [b' '; 11],
            parent: 0,
        }
    }

    fn dir_of(&self, e: &Entry) -> Result<Dir> {
        if !e.is_dir() {
            return Err(Error::NotDir);
        }
        Ok(if e.first_cluster == 0 { self.root() } else { Dir::Chain(e.first_cluster) })
    }

    /// The sectors of a directory.
    fn dir_sectors(&mut self, dir: Dir) -> Result<Vec<u32>> {
        match dir {
            Dir::FixedRoot => Ok((self.root_sector..self.data_sector).collect()),
            Dir::Chain(first) => {
                let mut out = Vec::new();
                for c in self.chain(first)? {
                    let s = self.cluster_sector(c);
                    out.extend(s..s + self.sectors_per_cluster);
                }
                Ok(out)
            }
        }
    }

    fn slot_bytes(&mut self, s: Slot) -> Result<[u8; 32]> {
        let b = self.cache.read(u64::from(s.sector))?;
        let mut out = [0; 32];
        out.copy_from_slice(&b[s.offset as usize..s.offset as usize + 32]);
        Ok(out)
    }

    fn write_slot(&mut self, s: Slot, bytes: &[u8; 32]) -> Result<()> {
        let at = s.offset as usize;
        self.cache.change(u64::from(s.sector), |b| b[at..at + 32].copy_from_slice(bytes))?;
        Ok(())
    }

    /// The entries of a directory: `.` and `..` left out of none, as on disk.
    pub fn read_dir(&mut self, dir: &Entry) -> Result<Vec<Entry>> {
        let d = self.dir_of(dir)?;
        let parent = if d == Dir::FixedRoot { 0 } else { dir.first_cluster };
        let mut out = Vec::new();
        let mut long: Vec<(u8, [u16; 13])> = Vec::new();
        let mut long_slots = Vec::new();
        let mut sum = 0u8;
        'all: for sector in self.dir_sectors(d)? {
            for offset in (0..self.bytes_per_sector).step_by(SLOT as usize) {
                let slot = Slot { sector, offset };
                let b = self.slot_bytes(slot)?;
                if b[0] == 0 {
                    break 'all;
                }
                if b[0] == DELETED {
                    long.clear();
                    long_slots.clear();
                    continue;
                }
                if b[11] & 0x3f == ATTR_LONG {
                    if b[0] & 0x40 != 0 {
                        long.clear();
                        long_slots.clear();
                        sum = b[13];
                    }
                    let mut units = [0u16; 13];
                    for (i, at) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() {
                        units[i] = u16_at(&b, *at);
                    }
                    long.push((b[0] & 0x1f, units));
                    long_slots.push(slot);
                    continue;
                }
                if b[11] & ATTR_VOLUME != 0 {
                    long.clear();
                    long_slots.clear();
                    continue;
                }
                // A long name counts only if it belongs to this short one.
                let complete = !long.is_empty()
                    && sum == checksum(&b)
                    && long.iter().rev().enumerate().all(|(i, p)| usize::from(p.0) == i + 1);
                let name = if complete {
                    let units: Vec<u16> =
                        long.iter().rev().flat_map(|p| p.1).take_while(|&u| u != 0 && u != 0xffff).collect();
                    String::from_utf16_lossy(&units)
                } else {
                    short_text(&b, b[12])
                };
                let mut short = [0; 11];
                short.copy_from_slice(&b[..11]);
                let high = if self.kind == Kind::Fat32 { u32::from(u16_at(&b, 20)) << 16 } else { 0 };
                out.push(Entry {
                    name,
                    attr: b[11],
                    first_cluster: high | u32::from(u16_at(&b, 26)),
                    size: u32::from_le_bytes([b[28], b[29], b[30], b[31]]),
                    mtime: DosTime { time: u16_at(&b, 22), date: u16_at(&b, 24) },
                    slot: Some(slot),
                    long: if complete { core::mem::take(&mut long_slots) } else { Vec::new() },
                    short,
                    parent,
                });
                long.clear();
                long_slots.clear();
            }
        }
        Ok(out)
    }

    /// `name` in directory `dir`.
    pub fn lookup(&mut self, dir: &Entry, name: &str) -> Result<Entry> {
        self.read_dir(dir)?.into_iter().find(|e| same(&e.name, name)).ok_or(Error::NotFound)
    }

    /// A path from the root.
    pub fn resolve(&mut self, path: &str) -> Result<Entry> {
        let mut at = self.root_entry();
        for part in path.split('/').filter(|p| !p.is_empty() && *p != ".") {
            at = self.lookup(&at, part)?;
        }
        Ok(at)
    }

    /// `path`'s directory and last name.
    fn parent_of(&mut self, path: &str) -> Result<(Entry, String)> {
        let path = path.trim_end_matches('/');
        let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
        let parent = self.resolve(dir)?;
        if !parent.is_dir() {
            return Err(Error::NotDir);
        }
        Ok((parent, name.into()))
    }

    /// `count` free slots in a row, growing the directory if need be.
    fn free_slots(&mut self, dir: Dir, count: usize) -> Result<Vec<Slot>> {
        let mut run = Vec::new();
        let sectors = self.dir_sectors(dir)?;
        for &sector in &sectors {
            for offset in (0..self.bytes_per_sector).step_by(SLOT as usize) {
                let slot = Slot { sector, offset };
                let first = self.slot_bytes(slot)?[0];
                if first == 0 || first == DELETED {
                    run.push(slot);
                    if run.len() == count {
                        return Ok(run);
                    }
                } else {
                    run.clear();
                }
            }
        }
        let Dir::Chain(first) = dir else { return Err(Error::NoSpace) };
        let last = *self.chain(first)?.last().ok_or(Error::Io)?;
        while run.len() < count {
            let c = self.new_cluster()?;
            let tail = self.last_of(last)?;
            self.set_next(tail, c)?;
            let s = self.cluster_sector(c);
            for sector in s..s + self.sectors_per_cluster {
                for offset in (0..self.bytes_per_sector).step_by(SLOT as usize) {
                    if run.len() < count {
                        run.push(Slot { sector, offset });
                    }
                }
            }
        }
        Ok(run)
    }

    fn last_of(&mut self, from: u32) -> Result<u32> {
        self.chain(from)?.last().copied().ok_or(Error::Io)
    }

    /// A zeroed cluster: a directory's must read as empty.
    fn new_cluster(&mut self) -> Result<u32> {
        let c = self.allocate()?;
        let s = self.cluster_sector(c);
        for sector in s..s + self.sectors_per_cluster {
            self.cache.change(u64::from(sector), |b| b.fill(0))?;
        }
        Ok(c)
    }

    fn short_entry(short: &[u8; 11], attr: u8, cluster: u32, size: u32, t: DosTime) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[..11].copy_from_slice(short);
        b[11] = attr;
        b[14..16].copy_from_slice(&t.time.to_le_bytes());
        b[16..18].copy_from_slice(&t.date.to_le_bytes());
        b[18..20].copy_from_slice(&t.date.to_le_bytes());
        b[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        b[22..24].copy_from_slice(&t.time.to_le_bytes());
        b[24..26].copy_from_slice(&t.date.to_le_bytes());
        b[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        b[28..32].copy_from_slice(&size.to_le_bytes());
        b
    }

    /// A new entry `name` in `dir`.
    fn add_entry(&mut self, parent: &Entry, name: &str, attr: u8, cluster: u32, size: u32, t: DosTime) -> Result<Entry> {
        if invalid(name) {
            return Err(if name.encode_utf16().count() > crate::name::LONG_MAX {
                Error::NameTooLong
            } else {
                Error::Invalid
            });
        }
        let present = self.read_dir(parent)?;
        if present.iter().any(|e| same(&e.name, name)) {
            return Err(Error::Exists);
        }
        let short = match exact_short(name) {
            // Free too among the short names of long ones.
            Some(s) if !present.iter().any(|e| e.short == s) => (s, false),
            Some(_) => {
                let taken = |s: &[u8; 11]| present.iter().any(|e| e.short == *s);
                let n = (1..1_000_000).find(|n| !taken(&numbered_short(name, *n))).ok_or(Error::NoSpace)?;
                (numbered_short(name, n), true)
            }
            None => {
                let taken = |s: &[u8; 11]| present.iter().any(|e| e.short == *s);
                let n = (1..1_000_000).find(|n| !taken(&numbered_short(name, *n))).ok_or(Error::NoSpace)?;
                (numbered_short(name, n), true)
            }
        };
        let units: Vec<u16> = name.encode_utf16().collect();
        let long_count = if short.1 { units.len().div_ceil(13) } else { 0 };
        let dir = self.dir_of(parent)?;
        let slots = self.free_slots(dir, long_count + 1)?;
        let sum = checksum(&short.0);
        for (i, slot) in slots[..long_count].iter().enumerate() {
            let ordinal = long_count - i;
            let mut b = [0xffu8; 32];
            b[0] = ordinal as u8 | if i == 0 { 0x40 } else { 0 };
            b[11] = ATTR_LONG;
            b[12] = 0;
            b[13] = sum;
            b[26] = 0;
            b[27] = 0;
            for (k, at) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() {
                let index = (ordinal - 1) * 13 + k;
                let u = match index.cmp(&units.len()) {
                    core::cmp::Ordering::Less => units[index],
                    core::cmp::Ordering::Equal => 0,
                    core::cmp::Ordering::Greater => 0xffff,
                };
                b[*at..at + 2].copy_from_slice(&u.to_le_bytes());
            }
            self.write_slot(*slot, &b)?;
        }
        let slot = slots[long_count];
        self.write_slot(slot, &Self::short_entry(&short.0, attr, cluster, size, t))?;
        Ok(Entry {
            name: name.into(),
            attr,
            first_cluster: cluster,
            size,
            mtime: t,
            slot: Some(slot),
            long: slots[..long_count].to_vec(),
            short: short.0,
            parent: if dir == Dir::FixedRoot { 0 } else { parent.first_cluster },
        })
    }

    /// A new empty file.
    pub fn create(&mut self, path: &str, t: DosTime) -> Result<Entry> {
        let (parent, name) = self.parent_of(path)?;
        self.add_entry(&parent, &name, ATTR_ARCHIVE, 0, 0, t)
    }

    /// A new directory, with its `.` and `..`.
    pub fn mkdir(&mut self, path: &str, t: DosTime) -> Result<Entry> {
        let (parent, name) = self.parent_of(path)?;
        if self.lookup(&parent, &name).is_ok() {
            return Err(Error::Exists);
        }
        let c = self.new_cluster()?;
        let entry = match self.add_entry(&parent, &name, ATTR_DIR, c, 0, t) {
            Ok(e) => e,
            Err(e) => {
                self.free_chain(c)?;
                return Err(e);
            }
        };
        let s = self.cluster_sector(c);
        // `..` of a directory in the root says 0, on FAT32 too.
        let up = if parent.first_cluster == self.root_cluster || parent.first_cluster == 0 { 0 } else { parent.first_cluster };
        self.write_slot(Slot { sector: s, offset: 0 }, &Self::short_entry(b".          ", ATTR_DIR, c, 0, t))?;
        self.write_slot(Slot { sector: s, offset: 32 }, &Self::short_entry(b"..         ", ATTR_DIR, up, 0, t))?;
        Ok(entry)
    }

    fn update_entry(&mut self, e: &Entry, t: Option<DosTime>) -> Result<()> {
        let Some(slot) = e.slot else { return Ok(()) };
        let mut b = self.slot_bytes(slot)?;
        b[20..22].copy_from_slice(&((e.first_cluster >> 16) as u16).to_le_bytes());
        b[26..28].copy_from_slice(&(e.first_cluster as u16).to_le_bytes());
        b[28..32].copy_from_slice(&e.size.to_le_bytes());
        if let Some(t) = t {
            b[22..24].copy_from_slice(&t.time.to_le_bytes());
            b[24..26].copy_from_slice(&t.date.to_le_bytes());
            b[11] |= ATTR_ARCHIVE;
        }
        self.write_slot(slot, &b)
    }

    /// Reads from `offset`: what there is, up to `buf`'s size.
    pub fn read_at(&mut self, e: &Entry, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if e.is_dir() {
            return Err(Error::IsDir);
        }
        if offset >= u64::from(e.size) {
            return Ok(0);
        }
        let want = buf.len().min((u64::from(e.size) - offset) as usize);
        let clusters = self.chain(e.first_cluster)?;
        let cb = u64::from(self.cluster_bytes());
        let bps = u64::from(self.bytes_per_sector);
        let mut done = 0;
        while done < want {
            let at = offset + done as u64;
            let c = *clusters.get((at / cb) as usize).ok_or(Error::Io)?;
            let sector = self.cluster_sector(c) as u64 + (at % cb) / bps;
            let inner = (at % bps) as usize;
            let n = (want - done).min(bps as usize - inner);
            let b = self.cache.read(sector)?;
            buf[done..done + n].copy_from_slice(&b[inner..inner + n]);
            done += n;
        }
        Ok(done)
    }

    /// Makes room for `size` bytes, zeroing what is new. Without room for
    /// all of it, nothing is taken: a file's chain matches its size.
    fn extend(&mut self, e: &mut Entry, size: u32) -> Result<()> {
        let cb = self.cluster_bytes();
        let have = self.chain(e.first_cluster)?;
        let need = size.div_ceil(cb) as usize;
        let mut added = Vec::new();
        for _ in have.len()..need {
            match self.new_cluster() {
                Ok(c) => added.push(c),
                Err(err) => {
                    for c in added {
                        self.set_next(c, FREE)?;
                    }
                    return Err(err);
                }
            }
        }
        let mut last = have.last().copied();
        for c in added {
            match last {
                Some(l) => self.set_next(l, c)?,
                None => e.first_cluster = c,
            }
            last = Some(c);
        }
        Ok(())
    }

    /// Writes at `offset`; a gap reads as zeros, as `deextend()` leaves it.
    pub fn write_at(&mut self, e: &mut Entry, offset: u64, data: &[u8], t: DosTime) -> Result<usize> {
        if e.is_dir() {
            return Err(Error::IsDir);
        }
        let end = offset.checked_add(data.len() as u64).filter(|n| *n <= u64::from(u32::MAX)).ok_or(Error::Invalid)?;
        self.extend(e, end as u32)?;
        if offset > u64::from(e.size) {
            let (from, gap) = (u64::from(e.size), (offset - u64::from(e.size)) as usize);
            let zeros = vec![0u8; gap];
            self.put(e, from, &zeros)?;
        }
        self.put(e, offset, data)?;
        e.size = e.size.max(end as u32);
        self.update_entry(e, Some(t))?;
        Ok(data.len())
    }

    fn put(&mut self, e: &Entry, offset: u64, data: &[u8]) -> Result<()> {
        let clusters = self.chain(e.first_cluster)?;
        let cb = u64::from(self.cluster_bytes());
        let bps = u64::from(self.bytes_per_sector);
        let mut done = 0;
        while done < data.len() {
            let at = offset + done as u64;
            let c = *clusters.get((at / cb) as usize).ok_or(Error::Io)?;
            let sector = self.cluster_sector(c) as u64 + (at % cb) / bps;
            let inner = (at % bps) as usize;
            let n = (data.len() - done).min(bps as usize - inner);
            let piece = &data[done..done + n];
            self.cache.change(sector, |b| b[inner..inner + n].copy_from_slice(piece))?;
            done += n;
        }
        Ok(())
    }

    /// Sets the size, freeing or zeroing clusters: `ftruncate()`.
    pub fn truncate(&mut self, e: &mut Entry, size: u32, t: DosTime) -> Result<()> {
        if e.is_dir() {
            return Err(Error::IsDir);
        }
        if size > e.size {
            let gap = vec![0u8; (size - e.size) as usize];
            let at = u64::from(e.size);
            self.write_at(e, at, &gap, t)?;
            return Ok(());
        }
        let cb = self.cluster_bytes();
        let keep = size.div_ceil(cb) as usize;
        let chain = self.chain(e.first_cluster)?;
        if keep == 0 {
            self.free_chain(e.first_cluster)?;
            e.first_cluster = FREE;
        } else if let Some(&next) = chain.get(keep) {
            let end = self.end_mark();
            self.set_next(chain[keep - 1], end)?;
            self.free_chain(next)?;
        }
        e.size = size;
        self.update_entry(e, Some(t))
    }

    /// `msdosfs_setattr()`'s `va_mode`: the owner's write bit is the
    /// read-only attribute; a directory has none to change, and the root
    /// no entry to change.
    pub fn set_mode(&mut self, path: &str, mode: u32) -> Result<()> {
        const S_IWUSR: u32 = 0o200;
        let e = self.resolve(path)?;
        let Some(slot) = e.slot else { return Err(Error::Invalid) };
        if e.is_dir() {
            return Ok(());
        }
        let mut b = self.slot_bytes(slot)?;
        if mode & S_IWUSR != 0 {
            b[11] &= !ATTR_READONLY;
        } else {
            b[11] |= ATTR_READONLY;
        }
        b[11] |= ATTR_ARCHIVE;
        self.write_slot(slot, &b)
    }

    /// `msdosfs_setattr()`'s `va_mtime`: the archive bit too, but on a
    /// directory, as Windows does.
    pub fn set_mtime(&mut self, path: &str, t: DosTime) -> Result<()> {
        let e = self.resolve(path)?;
        let Some(slot) = e.slot else { return Err(Error::Invalid) };
        let mut b = self.slot_bytes(slot)?;
        b[22..24].copy_from_slice(&t.time.to_le_bytes());
        b[24..26].copy_from_slice(&t.date.to_le_bytes());
        if !e.is_dir() {
            b[11] |= ATTR_ARCHIVE;
        }
        self.write_slot(slot, &b)
    }

    fn remove_slots(&mut self, e: &Entry) -> Result<()> {
        for &s in e.long.iter().chain(e.slot.iter()) {
            let mut b = self.slot_bytes(s)?;
            b[0] = DELETED;
            self.write_slot(s, &b)?;
        }
        Ok(())
    }

    /// `unlink()`.
    pub fn unlink(&mut self, path: &str) -> Result<()> {
        let e = self.resolve(path)?;
        if e.is_dir() {
            return Err(Error::IsDir);
        }
        self.remove_slots(&e)?;
        if e.first_cluster != 0 {
            self.free_chain(e.first_cluster)?;
        }
        Ok(())
    }

    /// `rmdir()`.
    pub fn rmdir(&mut self, path: &str) -> Result<()> {
        let e = self.resolve(path)?;
        if !e.is_dir() {
            return Err(Error::NotDir);
        }
        if e.slot.is_none() {
            return Err(Error::Busy);
        }
        if self.read_dir(&e)?.iter().any(|c| c.name != "." && c.name != "..") {
            return Err(Error::NotEmpty);
        }
        self.remove_slots(&e)?;
        self.free_chain(e.first_cluster)
    }

    /// `rename()`, also replacing a file or an empty directory.
    pub fn rename(&mut self, from: &str, to: &str, t: DosTime) -> Result<()> {
        let e = self.resolve(from)?;
        if e.slot.is_none() {
            return Err(Error::Busy);
        }
        let (parent, name) = self.parent_of(to)?;
        // Into itself: EINVAL, as a directory can't be below itself.
        if e.is_dir() {
            let mut at = parent.clone();
            loop {
                if at.first_cluster == e.first_cluster {
                    return Err(Error::Invalid);
                }
                if at.slot.is_none() || at.parent == 0 {
                    break;
                }
                let up = self.read_dir(&at)?.into_iter().find(|c| c.name == "..");
                match up {
                    Some(u) if u.first_cluster != 0 => {
                        at = Entry { first_cluster: u.first_cluster, attr: ATTR_DIR, ..u };
                    }
                    _ => break,
                }
            }
        }
        match self.lookup(&parent, &name) {
            Ok(old) if old.slot == e.slot => {
                // Only the case changes: the same entry, a new name.
                self.remove_slots(&e)?;
                self.add_entry(&parent, &name, e.attr, e.first_cluster, e.size, e.mtime)?;
                return Ok(());
            }
            Ok(old) if old.is_dir() != e.is_dir() => {
                return Err(if old.is_dir() { Error::IsDir } else { Error::NotDir });
            }
            Ok(old) if old.is_dir() => self.rmdir(to)?,
            Ok(_) => self.unlink(to)?,
            Err(Error::NotFound) => {}
            Err(err) => return Err(err),
        }
        let new = self.add_entry(&parent, &name, e.attr, e.first_cluster, e.size, t)?;
        self.remove_slots(&e)?;
        if e.is_dir() {
            // Its `..` follows it.
            let up = if parent.first_cluster == self.root_cluster || parent.first_cluster == 0 {
                0
            } else {
                parent.first_cluster
            };
            if let Some(dots) = self.read_dir(&new)?.into_iter().find(|c| c.name == "..")
                && let Some(slot) = dots.slot
            {
                let mut b = self.slot_bytes(slot)?;
                b[20..22].copy_from_slice(&((up >> 16) as u16).to_le_bytes());
                b[26..28].copy_from_slice(&(up as u16).to_le_bytes());
                self.write_slot(slot, &b)?;
            }
        }
        Ok(())
    }
}

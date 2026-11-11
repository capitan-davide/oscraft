//! FAT12, FAT16 and FAT32, as FreeBSD's `sys/fs/msdosfs` behaves: long
//! names, case kept but not significant, no owners or links. A library
//! over `bufcache`, so it runs on the host against images that real tools
//! make and check (`tests/`); `diskd` serves it.

mod dir;
mod fat;
mod name;

use std::io;

use bufcache::{BlockDevice, Cache};

pub use dir::{ATTR_DIR, ATTR_READONLY, Entry};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Error {
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotEmpty,
    NoSpace,
    NameTooLong,
    Invalid,
    Busy,
    Io,
}

impl From<io::Error> for Error {
    fn from(_: io::Error) -> Self {
        Error::Io
    }
}

pub type Result<T> = core::result::Result<T, Error>;

/// A time as FAT keeps it: date and time words, local time.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DosTime {
    pub date: u16,
    pub time: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Fat12,
    Fat16,
    Fat32,
}

pub struct Volume<D> {
    pub(crate) cache: Cache<D>,
    pub(crate) kind: Kind,
    pub(crate) bytes_per_sector: u32,
    pub(crate) sectors_per_cluster: u32,
    pub(crate) reserved: u32,
    pub(crate) fats: u32,
    pub(crate) fat_sectors: u32,
    pub(crate) root_sector: u32,
    pub(crate) data_sector: u32,
    pub(crate) clusters: u32,
    pub(crate) root_cluster: u32,
    pub(crate) next_free: u32,
    pub(crate) fsinfo: u32,
}

fn u16_at(b: &[u8], at: usize) -> u32 {
    u32::from(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl<D: BlockDevice> Volume<D> {
    /// Whether the disk holds a FAT file system.
    pub fn recognises(cache: &mut Cache<D>) -> bool {
        let block = cache.block_size() as u32;
        cache.read(0).is_ok_and(|b| {
            b.len() >= 512
                && b[510] == 0x55
                && b[511] == 0xaa
                && u16_at(b, 11) == block
                && u32::from(b[13]).is_power_of_two()
                && b[16] != 0
                && (u16_at(b, 22) != 0 || u32_at(b, 36) != 0)
        })
    }

    /// Reads the boot sector, as `mountmsdosfs()`.
    pub fn mount(cache: Cache<D>) -> Result<Self> {
        let mut cache = cache;
        let b = cache.read(0)?.to_vec();
        if b.len() < 512 || b[510] != 0x55 || b[511] != 0xaa {
            return Err(Error::Invalid);
        }
        let bytes_per_sector = u16_at(&b, 11);
        let sectors_per_cluster = u32::from(b[13]);
        let reserved = u16_at(&b, 14);
        let fats = u32::from(b[16]);
        let root_entries = u16_at(&b, 17);
        let total = match u16_at(&b, 19) {
            0 => u32_at(&b, 32),
            n => n,
        };
        let fat_sectors = match u16_at(&b, 22) {
            0 => u32_at(&b, 36),
            n => n,
        };
        if bytes_per_sector as usize != cache.block_size()
            || !bytes_per_sector.is_power_of_two()
            || !sectors_per_cluster.is_power_of_two()
            || fats == 0
            || fat_sectors == 0
        {
            return Err(Error::Invalid);
        }
        let root_sector = reserved + fats * fat_sectors;
        let root_sectors = (root_entries * 32).div_ceil(bytes_per_sector);
        let data_sector = root_sector + root_sectors;
        let clusters = total.checked_sub(data_sector).ok_or(Error::Invalid)? / sectors_per_cluster;
        // The FAT's type follows from the cluster count alone.
        let kind = match clusters {
            0..4085 => Kind::Fat12,
            4085..65525 => Kind::Fat16,
            _ => Kind::Fat32,
        };
        let (root_cluster, fsinfo) = if kind == Kind::Fat32 { (u32_at(&b, 44), u16_at(&b, 48)) } else { (0, 0) };
        Ok(Self {
            cache,
            kind,
            bytes_per_sector,
            sectors_per_cluster,
            reserved,
            fats,
            fat_sectors,
            root_sector,
            data_sector,
            clusters,
            root_cluster,
            next_free: 2,
            fsinfo,
        })
    }

    pub(crate) fn cluster_bytes(&self) -> u32 {
        self.sectors_per_cluster * self.bytes_per_sector
    }

    pub(crate) fn cluster_sector(&self, cluster: u32) -> u32 {
        self.data_sector + (cluster - 2) * self.sectors_per_cluster
    }

    /// What is on the disk is what `read` says; everything is written.
    pub fn sync(&mut self) -> Result<()> {
        self.write_fsinfo()?;
        self.cache.sync()?;
        Ok(())
    }

    /// The disk, given back.
    pub fn unmount(mut self) -> Result<D> {
        self.sync()?;
        Ok(self.cache.into_device())
    }

    /// Clusters, and those free: `statfs()`.
    pub fn space(&mut self) -> Result<(u32, u32, u32)> {
        let free = self.free_clusters()?;
        Ok((self.cluster_bytes(), self.clusters, free))
    }
}

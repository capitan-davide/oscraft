//! The file allocation table, as `msdosfs_fat.c`: reading and changing
//! entries in every copy, chains, and free clusters.

use bufcache::BlockDevice;

use crate::{Error, Kind, Result, Volume};

pub(crate) const FREE: u32 = 0;

impl<D: BlockDevice> Volume<D> {
    fn fat_byte(&mut self, copy: u32, offset: u32) -> Result<u8> {
        let sector = self.reserved + copy * self.fat_sectors + offset / self.bytes_per_sector;
        let at = (offset % self.bytes_per_sector) as usize;
        Ok(self.cache.read(u64::from(sector))?[at])
    }

    fn set_fat_byte(&mut self, offset: u32, value: u8) -> Result<()> {
        for copy in 0..self.fats {
            let sector = self.reserved + copy * self.fat_sectors + offset / self.bytes_per_sector;
            let at = (offset % self.bytes_per_sector) as usize;
            self.cache.change(u64::from(sector), |b| b[at] = value)?;
        }
        Ok(())
    }

    /// The entry of `cluster`.
    pub(crate) fn next(&mut self, cluster: u32) -> Result<u32> {
        match self.kind {
            Kind::Fat12 => {
                let at = cluster + cluster / 2;
                let v = u32::from(self.fat_byte(0, at)?) | u32::from(self.fat_byte(0, at + 1)?) << 8;
                Ok(if cluster & 1 != 0 { v >> 4 } else { v & 0xfff })
            }
            Kind::Fat16 => {
                let at = cluster * 2;
                Ok(u32::from(self.fat_byte(0, at)?) | u32::from(self.fat_byte(0, at + 1)?) << 8)
            }
            Kind::Fat32 => {
                let at = cluster * 4;
                let mut v = 0;
                for i in 0..4 {
                    v |= u32::from(self.fat_byte(0, at + i)?) << (8 * i);
                }
                Ok(v & 0x0fff_ffff)
            }
        }
    }

    pub(crate) fn set_next(&mut self, cluster: u32, value: u32) -> Result<()> {
        match self.kind {
            Kind::Fat12 => {
                let at = cluster + cluster / 2;
                let old = u32::from(self.fat_byte(0, at)?) | u32::from(self.fat_byte(0, at + 1)?) << 8;
                let v = if cluster & 1 != 0 {
                    (old & 0xf) | value << 4
                } else {
                    (old & 0xf000) | (value & 0xfff)
                };
                self.set_fat_byte(at, v as u8)?;
                self.set_fat_byte(at + 1, (v >> 8) as u8)
            }
            Kind::Fat16 => {
                let at = cluster * 2;
                self.set_fat_byte(at, value as u8)?;
                self.set_fat_byte(at + 1, (value >> 8) as u8)
            }
            Kind::Fat32 => {
                // The top four bits are reserved: they stay.
                let at = cluster * 4;
                let top = u32::from(self.fat_byte(0, at + 3)?) & 0xf0;
                self.set_fat_byte(at, value as u8)?;
                self.set_fat_byte(at + 1, (value >> 8) as u8)?;
                self.set_fat_byte(at + 2, (value >> 16) as u8)?;
                self.set_fat_byte(at + 3, top as u8 | ((value >> 24) & 0x0f) as u8)
            }
        }
    }

    pub(crate) fn end_mark(&self) -> u32 {
        match self.kind {
            Kind::Fat12 => 0xfff,
            Kind::Fat16 => 0xffff,
            Kind::Fat32 => 0x0fff_ffff,
        }
    }

    pub(crate) fn is_end(&self, value: u32) -> bool {
        value < 2 || value >= self.end_mark() & !7
    }

    /// The clusters of a chain.
    pub(crate) fn chain(&mut self, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        while !self.is_end(c) {
            if c >= self.clusters + 2 || out.len() as u32 > self.clusters {
                return Err(Error::Io);
            }
            out.push(c);
            c = self.next(c)?;
        }
        Ok(out)
    }

    /// A free cluster, marked as the end of a chain.
    pub(crate) fn allocate(&mut self) -> Result<u32> {
        let last = self.clusters + 2;
        let start = self.next_free.clamp(2, last - 1);
        for offset in 0..self.clusters {
            let c = 2 + (start - 2 + offset) % self.clusters;
            if self.next(c)? == FREE {
                let end = self.end_mark();
                self.set_next(c, end)?;
                self.next_free = c + 1;
                return Ok(c);
            }
        }
        Err(Error::NoSpace)
    }

    /// Frees a chain.
    pub(crate) fn free_chain(&mut self, first: u32) -> Result<()> {
        for c in self.chain(first)? {
            self.set_next(c, FREE)?;
            self.next_free = self.next_free.min(c);
        }
        Ok(())
    }

    pub(crate) fn free_clusters(&mut self) -> Result<u32> {
        let mut n = 0;
        for c in 2..self.clusters + 2 {
            n += u32::from(self.next(c)? == FREE);
        }
        Ok(n)
    }

    /// FAT32's hint of what is free.
    pub(crate) fn write_fsinfo(&mut self) -> Result<()> {
        if self.kind != Kind::Fat32 || self.fsinfo == 0 {
            return Ok(());
        }
        let free = self.free_clusters()?;
        let next = self.next_free;
        self.cache.change(u64::from(self.fsinfo), |b| {
            b[488..492].copy_from_slice(&free.to_le_bytes());
            b[492..496].copy_from_slice(&next.to_le_bytes());
        })?;
        Ok(())
    }
}

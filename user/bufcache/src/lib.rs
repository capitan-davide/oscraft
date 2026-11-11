//! A block device and its cache, for `diskd`: what FreeBSD's buffer cache
//! does for its file systems, and `io-blk.so` for QNX's. A file system
//! reads and changes blocks here; a changed block goes to the device when
//! the cache needs its room, or at `sync()`.

use std::collections::HashMap;
use std::io;

// ANCHOR: block_device
/// A disk: `block_size()` bytes at a time.
pub trait BlockDevice {
    fn block_size(&self) -> usize;
    fn blocks(&self) -> u64;
    fn read(&mut self, block: u64, buf: &mut [u8]) -> io::Result<()>;
    fn write(&mut self, block: u64, buf: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}
// ANCHOR_END: block_device

struct Buffer {
    data: Vec<u8>,
    dirty: bool,
    used: u64,
}

pub struct Cache<D> {
    device: D,
    buffers: HashMap<u64, Buffer>,
    capacity: usize,
    clock: u64,
}

impl<D: BlockDevice> Cache<D> {
    /// A cache of at most `capacity` blocks.
    pub fn new(device: D, capacity: usize) -> Self {
        Self { device, buffers: HashMap::new(), capacity: capacity.max(1), clock: 0 }
    }

    pub fn block_size(&self) -> usize {
        self.device.block_size()
    }

    pub fn blocks(&self) -> u64 {
        self.device.blocks()
    }

    /// Writes one block out to make room: the least recently used.
    fn make_room(&mut self) -> io::Result<()> {
        if self.buffers.len() < self.capacity {
            return Ok(());
        }
        let Some((&oldest, _)) = self.buffers.iter().min_by_key(|(_, b)| b.used) else {
            return Ok(());
        };
        if let Some(b) = self.buffers.get(&oldest).filter(|b| b.dirty) {
            self.device.write(oldest, &b.data)?;
        }
        self.buffers.remove(&oldest);
        Ok(())
    }

    fn buffer(&mut self, block: u64) -> io::Result<&mut Buffer> {
        if block >= self.device.blocks() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        self.clock += 1;
        let clock = self.clock;
        if !self.buffers.contains_key(&block) {
            self.make_room()?;
            let mut data = vec![0; self.device.block_size()];
            self.device.read(block, &mut data)?;
            self.buffers.insert(block, Buffer { data, dirty: false, used: clock });
        }
        let b = self.buffers.get_mut(&block).ok_or(io::ErrorKind::NotFound)?;
        b.used = clock;
        Ok(b)
    }

    /// The block's contents.
    pub fn read(&mut self, block: u64) -> io::Result<&[u8]> {
        Ok(&self.buffer(block)?.data)
    }

    /// Changes the block; it reaches the device later.
    pub fn change<R>(&mut self, block: u64, f: impl FnOnce(&mut [u8]) -> R) -> io::Result<R> {
        let b = self.buffer(block)?;
        b.dirty = true;
        Ok(f(&mut b.data))
    }

    /// Everything changed, on the device, in block order.
    pub fn sync(&mut self) -> io::Result<()> {
        let mut dirty: Vec<u64> = self.buffers.iter().filter(|(_, b)| b.dirty).map(|(n, _)| *n).collect();
        dirty.sort_unstable();
        for n in dirty {
            if let Some(b) = self.buffers.get_mut(&n) {
                self.device.write(n, &b.data)?;
                b.dirty = false;
            }
        }
        self.device.flush()
    }

    /// The device, given up: for its owner, after a `sync()`.
    pub fn into_device(self) -> D {
        self.device
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Disk {
        data: Vec<u8>,
        reads: u32,
        writes: u32,
    }

    impl BlockDevice for Disk {
        fn block_size(&self) -> usize {
            16
        }
        fn blocks(&self) -> u64 {
            (self.data.len() / 16) as u64
        }
        fn read(&mut self, block: u64, buf: &mut [u8]) -> io::Result<()> {
            self.reads += 1;
            let at = block as usize * 16;
            buf.copy_from_slice(&self.data[at..at + 16]);
            Ok(())
        }
        fn write(&mut self, block: u64, buf: &[u8]) -> io::Result<()> {
            self.writes += 1;
            let at = block as usize * 16;
            self.data[at..at + 16].copy_from_slice(buf);
            Ok(())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn disk(blocks: usize) -> Disk {
        Disk { data: vec![0; blocks * 16], ..Disk::default() }
    }

    #[test]
    fn a_block_is_read_once() {
        let mut c = Cache::new(disk(4), 4);
        c.read(1).unwrap();
        c.read(1).unwrap();
        assert_eq!(c.into_device().reads, 1);
    }

    #[test]
    fn a_change_waits_for_sync() {
        let mut c = Cache::new(disk(4), 4);
        c.change(2, |b| b[0] = 7).unwrap();
        assert_eq!(c.read(2).unwrap()[0], 7);
        assert_eq!(c.device.writes, 0);
        c.sync().unwrap();
        c.sync().unwrap();
        let d = c.into_device();
        assert_eq!((d.writes, d.data[32]), (1, 7));
    }

    #[test]
    fn the_oldest_goes_and_is_written_if_changed() {
        let mut c = Cache::new(disk(8), 2);
        c.change(0, |b| b[0] = 1).unwrap();
        c.read(1).unwrap();
        c.read(2).unwrap();
        assert_eq!(c.device.data[0], 1, "evicted, written");
        assert_eq!(c.read(0).unwrap()[0], 1);
    }

    #[test]
    fn past_the_end_is_an_error() {
        let mut c = Cache::new(disk(2), 2);
        assert!(c.read(2).is_err());
    }
}

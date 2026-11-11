//! `md`, FreeBSD's memory disk: blocks in memory, or in a file.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

use bufcache::BlockDevice;

pub const BLOCK: usize = 512;

/// `mdconfig -t malloc`.
pub struct Memory(Vec<u8>);

impl Memory {
    pub fn new(bytes: usize) -> Self {
        Self(vec![0; bytes / BLOCK * BLOCK])
    }
}

impl BlockDevice for Memory {
    fn block_size(&self) -> usize {
        BLOCK
    }
    fn blocks(&self) -> u64 {
        (self.0.len() / BLOCK) as u64
    }
    fn read(&mut self, block: u64, buf: &mut [u8]) -> io::Result<()> {
        let at = block as usize * BLOCK;
        buf.copy_from_slice(&self.0[at..at + BLOCK]);
        Ok(())
    }
    fn write(&mut self, block: u64, buf: &[u8]) -> io::Result<()> {
        let at = block as usize * BLOCK;
        self.0[at..at + BLOCK].copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// `mdconfig -t vnode -f file`.
pub struct Image {
    file: File,
    blocks: u64,
}

impl Image {
    pub fn new(file: File) -> io::Result<Self> {
        let blocks = file.metadata()?.len() / BLOCK as u64;
        Ok(Self { file, blocks })
    }
}

impl BlockDevice for Image {
    fn block_size(&self) -> usize {
        BLOCK
    }
    fn blocks(&self) -> u64 {
        self.blocks
    }
    fn read(&mut self, block: u64, buf: &mut [u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(block * BLOCK as u64))?;
        self.file.read_exact(buf)
    }
    fn write(&mut self, block: u64, buf: &[u8]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(block * BLOCK as u64))?;
        self.file.write_all(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_keeps_blocks() {
        let mut m = Memory::new(4096);
        assert_eq!(m.blocks(), 8);
        m.write(3, &[7; BLOCK]).unwrap();
        let mut b = [0; BLOCK];
        m.read(3, &mut b).unwrap();
        assert_eq!(b, [7; BLOCK]);
    }

    #[test]
    fn an_image_is_read_and_written_in_place() {
        let path = std::env::temp_dir().join(format!("md-test-{}", std::process::id()));
        let f = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
        f.set_len(1024).unwrap();
        let mut i = Image::new(f).unwrap();
        i.write(1, &[9; BLOCK]).unwrap();
        i.flush().unwrap();
        assert_eq!(std::fs::read(&path).unwrap()[512], 9);
        std::fs::remove_file(path).unwrap();
    }
}

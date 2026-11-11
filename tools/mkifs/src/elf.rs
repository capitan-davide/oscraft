//! Just enough ELF64 (little-endian) to place executables in a boot image.

use anyhow::{Context, Result, bail, ensure};

use crate::format::{
    IFS_BOOTSTRAP_DATA_ARGS, IFS_BOOTSTRAP_HEAD_SIZE, IFS_BOOTSTRAP_SIGNATURE, get_u16, get_u32,
    get_u64, put_u16, put_u32, put_u64,
};

pub const ET_REL: u16 = 1;
pub const ET_EXEC: u16 = 2;
pub const ET_DYN: u16 = 3;
pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
const DT_NULL: u64 = 0;
const DT_STRTAB: u64 = 5;
const DT_SONAME: u64 = 14;

pub const PAGE_SIZE: u64 = 0x1000;
const EHDR_SIZE: usize = 64;
const PHDR_SIZE: usize = 56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

#[derive(Debug, Clone)]
pub struct Elf {
    pub data: Vec<u8>,
    pub e_type: u16,
    pub e_machine: u16,
    pub e_entry: u64,
    pub e_flags: u32,
    pub phdrs: Vec<Phdr>,
}

/// True if `data` starts like a 64-bit little-endian ELF file.
pub fn is_elf64(data: &[u8]) -> bool {
    data.len() >= EHDR_SIZE && &data[..4] == b"\x7fELF" && data[4] == 2 && data[5] == 1
}

impl Elf {
    pub fn parse(data: Vec<u8>) -> Result<Self> {
        ensure!(is_elf64(&data), "not a 64-bit little-endian ELF file");
        let phoff = get_u64(&data, 32) as usize;
        let phentsize = get_u16(&data, 54) as usize;
        let phnum = get_u16(&data, 56) as usize;
        ensure!(
            phnum == 0 || phentsize == PHDR_SIZE,
            "unexpected program header size {phentsize}"
        );
        ensure!(
            phoff + phnum * PHDR_SIZE <= data.len(),
            "program headers out of bounds"
        );
        let phdrs = (0..phnum)
            .map(|i| {
                let o = phoff + i * PHDR_SIZE;
                Phdr {
                    p_type: get_u32(&data, o),
                    p_flags: get_u32(&data, o + 4),
                    p_offset: get_u64(&data, o + 8),
                    p_vaddr: get_u64(&data, o + 16),
                    p_paddr: get_u64(&data, o + 24),
                    p_filesz: get_u64(&data, o + 32),
                    p_memsz: get_u64(&data, o + 40),
                    p_align: get_u64(&data, o + 48),
                }
            })
            .collect();
        Ok(Self {
            e_type: get_u16(&data, 16),
            e_machine: get_u16(&data, 18),
            e_entry: get_u64(&data, 24),
            e_flags: get_u32(&data, 48),
            phdrs,
            data,
        })
    }

    /// Loadable segments with content in memory, sorted by virtual address.
    pub fn loads(&self) -> Vec<Phdr> {
        let mut v: Vec<_> = self
            .phdrs
            .iter()
            .copied()
            .filter(|p| p.p_type == PT_LOAD && p.p_memsz > 0)
            .collect();
        v.sort_by_key(|p| p.p_vaddr);
        v
    }

    /// File offset of a virtual address inside a loaded segment's file image.
    fn vaddr_to_offset(&self, vaddr: u64) -> Option<usize> {
        self.loads()
            .iter()
            .find(|p| vaddr >= p.p_vaddr && vaddr < p.p_vaddr + p.p_filesz)
            .map(|p| (p.p_offset + vaddr - p.p_vaddr) as usize)
    }

    /// `len` bytes of file-backed segment content at `vaddr`.
    pub fn read(&self, vaddr: u64, len: usize) -> Option<&[u8]> {
        let off = self.vaddr_to_offset(vaddr)?;
        self.data.get(off..off + len)
    }

    /// `DT_SONAME` of a shared object.
    pub fn soname(&self) -> Option<String> {
        let dynamic = self.phdrs.iter().find(|p| p.p_type == PT_DYNAMIC)?;
        let (mut strtab, mut soname) = (None, None);
        let start = dynamic.p_offset as usize;
        let end = (start + dynamic.p_filesz as usize).min(self.data.len());
        for o in (start..end.saturating_sub(15)).step_by(16) {
            match get_u64(&self.data, o) {
                DT_NULL => break,
                DT_STRTAB => strtab = Some(get_u64(&self.data, o + 8)),
                DT_SONAME => soname = Some(get_u64(&self.data, o + 8)),
                _ => {}
            }
        }
        let off = self.vaddr_to_offset(strtab?)? + soname? as usize;
        let s = self.data.get(off..)?;
        let n = s.iter().position(|&c| c == 0)?;
        Some(String::from_utf8_lossy(&s[..n]).into_owned())
    }
}

/// The memory image of an executable's loadable segments, with `.bss`
/// materialised as zeros.
struct MemImage {
    segments: Vec<(Phdr, Vec<u8>)>,
}

impl MemImage {
    fn new(elf: &Elf) -> Result<Self> {
        let segments = elf
            .loads()
            .into_iter()
            .map(|p| {
                let start = p.p_offset as usize;
                let end = start + p.p_filesz as usize;
                ensure!(
                    end <= elf.data.len(),
                    "segment at {:#x} out of file bounds",
                    p.p_vaddr
                );
                let mut bytes = elf.data[start..end].to_vec();
                bytes.resize(p.p_memsz as usize, 0);
                Ok((p, bytes))
            })
            .collect::<Result<_>>()?;
        Ok(Self { segments })
    }

    fn slice_mut(&mut self, vaddr: u64, len: usize) -> Option<&mut [u8]> {
        self.segments.iter_mut().find_map(|(p, bytes)| {
            let off = vaddr.checked_sub(p.p_vaddr)? as usize;
            bytes.get_mut(off..off + len)
        })
    }

    fn read_u64(&mut self, vaddr: u64) -> Option<u64> {
        self.slice_mut(vaddr, 8).map(|s| get_u64(s, 0))
    }
}

/// Write `bootargs` where a bootstrap executable asks for them: the entry
/// point is preceded by `struct ifs_bootstrap_head` ('BSHD', pointer to
/// `struct ifs_bootstrap_data`), whose `args` member points to the buffer.
fn patch_bootstrap_args(elf: &Elf, mem: &mut MemImage, bootargs: &[u8]) -> Result<()> {
    let head = elf.e_entry - IFS_BOOTSTRAP_HEAD_SIZE;
    let sig = mem.slice_mut(head, 4).map(|s| get_u32(s, 0));
    if sig != Some(IFS_BOOTSTRAP_SIGNATURE) {
        bail!(
            "no bootstrap head ('BSHD') before the entry point {:#x}",
            elf.e_entry
        );
    }
    let data = mem
        .read_u64(head + 8)
        .context("bootstrap head points outside the image")?;
    let args = mem
        .read_u64(data + IFS_BOOTSTRAP_DATA_ARGS)
        .context("bootstrap data outside the image")?;
    if args == 0 {
        bail!("bootstrap data at {data:#x} has no argument buffer");
    }
    let dst = mem.slice_mut(args, bootargs.len()).with_context(|| {
        format!(
            "argument buffer at {args:#x} too small for {} bytes",
            bootargs.len()
        )
    })?;
    dst.copy_from_slice(bootargs);
    Ok(())
}

/// Turn a linked bootstrap executable into the form startup loads in place:
/// each segment page-aligned in the file, `.bss` zero-filled (`filesz ==
/// memsz`), `p_paddr` = physical address of the segment in the image, and
/// no section headers. `paddr` is where the file itself will sit; the result
/// is padded to a whole number of pages.
pub fn process_bootstrap(elf: &Elf, paddr: u64, bootargs: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        elf.e_type == ET_EXEC,
        "bootstrap executable is not linked (ELF type {})",
        elf.e_type
    );
    ensure!(
        paddr.is_multiple_of(PAGE_SIZE),
        "bootstrap executable must be page aligned"
    );
    let mut mem = MemImage::new(elf)?;
    patch_bootstrap_args(elf, &mut mem, bootargs)?;

    let n = mem.segments.len();
    ensure!(
        EHDR_SIZE + n * PHDR_SIZE <= PAGE_SIZE as usize,
        "too many segments"
    );
    let mut out = vec![0u8; PAGE_SIZE as usize];
    let mut phdrs = Vec::with_capacity(n);
    for (p, bytes) in &mem.segments {
        let page = (out.len() as u64).next_multiple_of(PAGE_SIZE);
        let offset = page + p.p_vaddr % PAGE_SIZE;
        out.resize(offset as usize, 0);
        out.extend_from_slice(bytes);
        let size = bytes.len() as u64;
        phdrs.push(Phdr {
            p_offset: offset,
            p_paddr: paddr + offset,
            p_filesz: size,
            p_memsz: size,
            p_align: PAGE_SIZE,
            ..*p
        });
    }
    out.resize(out.len().next_multiple_of(PAGE_SIZE as usize), 0);

    // ELF header: identification from the input, executable, no sections.
    out[..16].copy_from_slice(&elf.data[..16]);
    put_u16(&mut out, 16, ET_EXEC);
    put_u16(&mut out, 18, elf.e_machine);
    put_u32(&mut out, 20, 1);
    put_u64(&mut out, 24, elf.e_entry);
    put_u64(&mut out, 32, EHDR_SIZE as u64);
    put_u64(&mut out, 40, 0);
    put_u32(&mut out, 48, elf.e_flags);
    put_u16(&mut out, 52, EHDR_SIZE as u16);
    put_u16(&mut out, 54, PHDR_SIZE as u16);
    put_u16(&mut out, 56, n as u16);
    for (i, p) in phdrs.iter().enumerate() {
        let o = EHDR_SIZE + i * PHDR_SIZE;
        put_u32(&mut out, o, PT_LOAD);
        put_u32(&mut out, o + 4, p.p_flags);
        put_u64(&mut out, o + 8, p.p_offset);
        put_u64(&mut out, o + 16, p.p_vaddr);
        put_u64(&mut out, o + 24, p.p_paddr);
        put_u64(&mut out, o + 32, p.p_filesz);
        put_u64(&mut out, o + 40, p.p_memsz);
        put_u64(&mut out, o + 48, p.p_align);
    }
    Ok(out)
}

/// Flatten a linked, physically addressed startup program into the bytes
/// that go right after the startup header: all loadable segments from
/// `start` up to the end of the last one, gaps and `.bss` zero-filled.
pub fn flatten(elf: &Elf, start: u64) -> Result<Vec<u8>> {
    ensure!(
        elf.e_type == ET_EXEC,
        "startup is not linked (ELF type {})",
        elf.e_type
    );
    let mem = MemImage::new(elf)?;
    let first = mem
        .segments
        .first()
        .context("startup has no loadable segments")?
        .0
        .p_vaddr;
    ensure!(
        first >= start,
        "startup segment at {first:#x} is below its link address {start:#x}"
    );
    let end = mem
        .segments
        .iter()
        .map(|(p, _)| p.p_vaddr + p.p_memsz)
        .max()
        .unwrap();
    let mut out = vec![0u8; (end - start) as usize];
    for (p, bytes) in &mem.segments {
        let o = (p.p_vaddr - start) as usize;
        out[o..o + bytes.len()].copy_from_slice(bytes);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny ET_EXEC with one RX and one RW segment and a bootstrap head.
    fn sample() -> Elf {
        let text_va = 0xffff_ff80_6003_1000u64;
        let data_va = 0xffff_ff80_600e_0000u64;
        let entry = text_va + 0x100;
        let bsdata = data_va + 0x10;
        let args = data_va + 0x40; // in .bss (filesz is 0x30)
        let mut f = vec![0u8; 0x3000];
        f[..4].copy_from_slice(b"\x7fELF");
        f[4] = 2;
        f[5] = 1;
        f[6] = 1;
        put_u16(&mut f, 16, ET_EXEC);
        put_u16(&mut f, 18, 183);
        put_u64(&mut f, 24, entry);
        put_u64(&mut f, 32, 64);
        put_u16(&mut f, 54, 56);
        put_u16(&mut f, 56, 2);
        let ph = |f: &mut Vec<u8>, i: usize, flags, off, va, filesz, memsz| {
            let o = 64 + i * 56;
            put_u32(f, o, PT_LOAD);
            put_u32(f, o + 4, flags);
            put_u64(f, o + 8, off);
            put_u64(f, o + 16, va);
            put_u64(f, o + 32, filesz);
            put_u64(f, o + 40, memsz);
        };
        ph(&mut f, 0, 5, 0x1000, text_va, 0x200, 0x200);
        ph(&mut f, 1, 6, 0x2000, data_va, 0x30, 0x100);
        // bootstrap head right before the entry point
        put_u32(&mut f, 0x1100 - 16, IFS_BOOTSTRAP_SIGNATURE);
        put_u64(&mut f, 0x1100 - 8, bsdata);
        // bootstrap data: size 0x20, args pointer
        put_u32(&mut f, 0x2010, 0x20);
        put_u64(&mut f, 0x2010 + 16, args);
        Elf::parse(f).unwrap()
    }

    #[test]
    fn processed_bootstrap_maps_in_place() {
        let elf = sample();
        let out = process_bootstrap(&elf, 0xb0000, b"ARGS").unwrap();
        let p = Elf::parse(out.clone()).unwrap();
        let loads = p.loads();
        assert_eq!(out.len() % 0x1000, 0);
        assert_eq!(loads[0].p_offset, 0x1000);
        assert_eq!(loads[0].p_paddr, 0xb1000);
        assert_eq!(loads[1].p_offset, 0x2000);
        assert_eq!(loads[1].p_paddr, 0xb2000);
        assert_eq!(loads[1].p_filesz, 0x100, ".bss materialised");
        assert_eq!(&out[0x2040..0x2044], b"ARGS");
        assert_eq!(get_u16(&out, 60), 0, "no section headers");
    }

    #[test]
    fn missing_bootstrap_head_is_an_error() {
        let mut elf = sample();
        elf.e_entry += 8;
        assert!(process_bootstrap(&elf, 0xb0000, b"x").is_err());
    }

    #[test]
    fn flatten_fills_gaps_and_bss() {
        let elf = sample();
        let flat = flatten(&elf, 0xffff_ff80_6003_1000).unwrap();
        assert_eq!(
            flat.len() as u64,
            0xffff_ff80_600e_0100 - 0xffff_ff80_6003_1000
        );
    }
}

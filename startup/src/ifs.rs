//! The image file system and the bootstrap executables in it.
//!
//! startup loads every file mkifs marked as a bootstrap executable, in
//! the order of the directory, links each to the next one through the
//! `ifs_bootstrap_data` the executable carries, and starts the first.
//! The kernel, `procnto`, is the last one (usually the only one).

use crate::abi::elf::{Elf64Ehdr, Elf64Phdr, PT_LOAD};
use crate::abi::image::*;
use crate::arch::aarch64::mmu::{Mmu, PAGE_SIZE, Prot};
use crate::elf::{self, Step};
use crate::kprintln;
use crate::ram::Ram;
use core::mem::size_of;

/// Read a value at a physical address (the MMU is off).
fn read<T: Copy>(paddr: u64) -> T {
    // SAFETY: the addresses are inside the image, or ones the page tables
    // translate to RAM startup allocated.
    unsafe { core::ptr::read_unaligned(paddr as *const T) }
}

/// The image file system's header at `paddr`, checked.
pub fn header(paddr: u64) -> ImageHeader {
    let h: ImageHeader = read(paddr);
    if h.signature != IMAGE_SIGNATURE {
        crate::crash!("no image file system at {paddr:#x}");
    }
    if h.flags & IMAGE_FLAGS_INO_BITS == 0 {
        crate::crash!("image file system too old (no inode bits)");
    }
    h
}

/// The NUL-terminated path at `paddr`, for messages.
fn path(paddr: u64) -> &'static str {
    // SAFETY: the path is in the image, which stays where it is.
    let bytes = unsafe { core::ffi::CStr::from_ptr(paddr as *const core::ffi::c_char) };
    bytes.to_str().unwrap_or("?")
}

// ANCHOR: load_one
/// Load the bootstrap executable whose ELF file is at `file`: map or copy
/// its segments, and return its entry point.
fn load_one(mmu: &mut Mmu, ram: &mut Ram, file: u64, name: &str) -> u64 {
    let ehdr: Elf64Ehdr = read(file);
    if let Err(e) = elf::check(&ehdr) {
        crate::crash!("/{name}: cannot load: {e:?}");
    }
    for i in 0..u64::from(ehdr.e_phnum) {
        let phdr: Elf64Phdr = read(file + ehdr.e_phoff + i * size_of::<Elf64Phdr>() as u64);
        if phdr.p_type != PT_LOAD {
            continue;
        }
        for step in elf::plan(&phdr, file).into_iter().flatten() {
            match step {
                Step::Map {
                    vaddr,
                    paddr,
                    size,
                    access,
                } => {
                    mmu.map(ram, Some(vaddr), paddr, size, prot(access));
                }
                Step::Alloc {
                    vaddr,
                    size,
                    src,
                    copy,
                    access,
                } => {
                    let Some(dst) = ram.alloc(size, PAGE_SIZE) else {
                        crate::crash!("/{name}: no RAM for a segment of {size:#x} bytes");
                    };
                    // SAFETY: dst is RAM startup just allocated, src is in
                    // the image; they do not overlap.
                    unsafe {
                        core::ptr::write_bytes(dst as *mut u8, 0, size as usize);
                        core::ptr::copy_nonoverlapping(
                            src as *const u8,
                            dst as *mut u8,
                            copy as usize,
                        );
                    }
                    mmu.map(ram, Some(vaddr), dst, size, prot(access));
                }
            }
        }
    }
    ehdr.e_entry
}
// ANCHOR_END: load_one

fn prot(a: elf::Access) -> Prot {
    Prot {
        write: a.write,
        exec: a.exec,
        ..Prot::RO
    }
}

// ANCHOR: load_bootstraps
/// Load the bootstrap executables of the image file system at `ifs`, link
/// each to the next, and return the first one's entry point.
pub fn load_bootstraps(mmu: &mut Mmu, ram: &mut Ram, ifs: u64, verbose: bool) -> u64 {
    let hdr = header(ifs);
    let mut dirent = ifs + u64::from(hdr.dir_offset);
    let mut first = None;
    // The previous executable's ifs_bootstrap_data, to link to this one.
    let mut prev_data: Option<u64> = None;
    loop {
        let attr: ImageAttr = read(dirent);
        if attr.size == 0 {
            break;
        }
        if attr.ino & IFS_INO_BOOTSTRAP_EXE != 0 {
            let f: ImageFile = read(dirent);
            let name = path(dirent + size_of::<ImageFile>() as u64);
            let entry = load_one(mmu, ram, ifs + u64::from(f.offset), name);
            if verbose {
                kprintln!("Loaded /{name}, entry {entry:#x}");
            }
            // The executable's head is right before its entry point.
            let head_va = entry - size_of::<IfsBootstrapHead>() as u64;
            let head: IfsBootstrapHead = read(translate(mmu, head_va, name));
            if head.signature != IFS_BOOTSTRAP_SIGNATURE {
                crate::crash!("/{name}: no bootstrap signature before its entry point");
            }
            let data = translate(mmu, head.bootstrap, name);
            match prev_data {
                Some(prev) => {
                    let next = prev + core::mem::offset_of!(IfsBootstrapData, next_entry) as u64;
                    // SAFETY: the previous executable's data, in RAM the
                    // page tables map.
                    unsafe { core::ptr::write_unaligned(next as *mut u64, entry) };
                }
                None => first = Some(entry),
            }
            prev_data = Some(data);
        }
        dirent += u64::from(attr.size);
    }
    first.unwrap_or_else(|| crate::crash!("no bootstrap executables in the image"))
}
// ANCHOR_END: load_bootstraps

fn translate(mmu: &Mmu, vaddr: u64, name: &str) -> u64 {
    mmu.vaddr_to_paddr(vaddr)
        .unwrap_or_else(|| crate::crash!("/{name}: {vaddr:#x} is not mapped"))
}

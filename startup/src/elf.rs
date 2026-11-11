//! Loading a bootstrap executable's ELF segments.
//!
//! mkifs links each bootstrap executable at its address in the kernel's
//! address space, and for the ones it keeps linked ("keeplinked", like
//! `procnto`), records in `p_paddr` where the segment's bytes are in the
//! image. Startup then maps those pages where they are, and gives RAM
//! only to what must be written to: the part of a segment beyond its file
//! bytes (`.bss`). A segment with no `p_paddr` is copied to new RAM.
//!
//! [`plan`] works out these steps from the program headers; the caller
//! carries them out with its page tables and RAM.

use crate::abi::elf::*;

const PAGE: u64 = 4096;

/// What a segment may be accessed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub write: bool,
    pub exec: bool,
}

impl Access {
    fn of(phdr: &Elf64Phdr) -> Self {
        Self {
            write: phdr.p_flags & PF_W != 0,
            exec: phdr.p_flags & PF_X != 0,
        }
    }
}

/// One step of loading a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Map `size` bytes of the image at `paddr` to `vaddr`, in place.
    Map {
        vaddr: u64,
        paddr: u64,
        size: u64,
        access: Access,
    },
    /// Allocate `size` bytes of zeroed RAM, copy `copy` bytes from the
    /// image at `src` to its start, and map it at `vaddr`.
    Alloc {
        vaddr: u64,
        size: u64,
        src: u64,
        copy: u64,
        access: Access,
    },
}

/// Why an ELF file cannot be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NotElf64,
    WrongMachine,
}

/// Check the ELF header of a bootstrap executable.
pub fn check(ehdr: &Elf64Ehdr) -> Result<(), Error> {
    let id = &ehdr.e_ident;
    if id[..4] != ELFMAG
        || id[4] != ELFCLASS64
        || id[5] != ELFDATA2LSB
        || ehdr.e_phnum == 0
        || usize::from(ehdr.e_phentsize) != core::mem::size_of::<Elf64Phdr>()
    {
        return Err(Error::NotElf64);
    }
    if ehdr.e_machine != EM_AARCH64 {
        return Err(Error::WrongMachine);
    }
    Ok(())
}

// ANCHOR: plan
/// The steps that load one segment, `phdr`, of the file at `file_paddr`
/// in the image: at most two (see the module documentation).
pub fn plan(phdr: &Elf64Phdr, file_paddr: u64) -> [Option<Step>; 2] {
    let access = Access::of(phdr);
    // Work in whole pages: the segment starts at the beginning of its
    // first page.
    let off = phdr.p_vaddr % PAGE;
    let vaddr = phdr.p_vaddr - off;
    let mut filesz = phdr.p_filesz + off;
    let mut memsz = phdr.p_memsz + off;
    // A read-only segment with no .bss may use the rest of its last page.
    if !access.write && memsz == filesz {
        filesz = filesz.next_multiple_of(PAGE);
        memsz = filesz;
    }
    if phdr.p_paddr == 0 {
        // Not linked in place: copy all of it to new RAM.
        let size = memsz.next_multiple_of(PAGE);
        let src = file_paddr + phdr.p_offset - off;
        return [
            Some(Step::Alloc {
                vaddr,
                size,
                src,
                copy: filesz,
                access,
            }),
            None,
        ];
    }
    let paddr = phdr.p_paddr - off;
    if phdr.p_memsz == phdr.p_filesz {
        filesz = filesz.next_multiple_of(PAGE);
        memsz = filesz;
    }
    // The whole pages of file bytes stay where they are...
    let in_place = filesz & !(PAGE - 1);
    let map = (in_place > 0).then_some(Step::Map {
        vaddr,
        paddr,
        size: in_place,
        access,
    });
    // ...and the partial last page and the .bss get new RAM.
    let rest = (memsz - in_place).next_multiple_of(PAGE);
    let alloc = (rest > 0).then_some(Step::Alloc {
        vaddr: vaddr + in_place,
        size: rest,
        src: paddr + in_place,
        copy: filesz - in_place,
        access,
    });
    [map, alloc]
}
// ANCHOR_END: plan

#[cfg(test)]
mod tests {
    use super::*;

    fn phdr(vaddr: u64, paddr: u64, offset: u64, filesz: u64, memsz: u64, flags: u32) -> Elf64Phdr {
        Elf64Phdr {
            p_type: PT_LOAD,
            p_flags: flags,
            p_offset: offset,
            p_vaddr: vaddr,
            p_paddr: paddr,
            p_filesz: filesz,
            p_memsz: memsz,
            p_align: PAGE,
        }
    }

    const RX: Access = Access {
        write: false,
        exec: true,
    };
    const RW: Access = Access {
        write: true,
        exec: false,
    };

    #[test]
    fn keeplinked_text_maps_in_place() {
        // procnto's text: 0xa25f8 bytes at 0x99000 in the image.
        let p = phdr(
            0xffff_ff80_6001_9000,
            0x99000,
            0x1000,
            0xa25f8,
            0xa25f8,
            PF_R | PF_X,
        );
        assert_eq!(
            plan(&p, 0x98000),
            [
                Some(Step::Map {
                    vaddr: 0xffff_ff80_6001_9000,
                    paddr: 0x99000,
                    size: 0xa3000,
                    access: RX
                }),
                None
            ]
        );
    }

    #[test]
    fn keeplinked_data_with_bss_gets_ram_for_the_tail() {
        // 0x1800 bytes of data, then 0x3000 bytes of bss.
        let p = phdr(
            0xffff_ff80_600c_0000,
            0x13c000,
            0xa4000,
            0x1800,
            0x4800,
            PF_R | PF_W,
        );
        assert_eq!(
            plan(&p, 0x98000),
            [
                Some(Step::Map {
                    vaddr: 0xffff_ff80_600c_0000,
                    paddr: 0x13c000,
                    size: 0x1000,
                    access: RW
                }),
                Some(Step::Alloc {
                    vaddr: 0xffff_ff80_600c_1000,
                    size: 0x4000,
                    src: 0x13d000,
                    copy: 0x800,
                    access: RW
                }),
            ]
        );
    }

    #[test]
    fn segment_without_paddr_is_copied() {
        let p = phdr(0xffff_ff80_6000_0100, 0, 0x100, 0x200, 0x300, PF_R | PF_W);
        assert_eq!(
            plan(&p, 0x20_0000),
            [
                Some(Step::Alloc {
                    vaddr: 0xffff_ff80_6000_0000,
                    size: 0x1000,
                    src: 0x20_0000,
                    copy: 0x300,
                    access: RW
                }),
                None
            ]
        );
    }
}

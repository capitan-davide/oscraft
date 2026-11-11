//! ELF loader (QNX `proc/loader_elf.c`): load a static AArch64 executable
//! into a process's address space and give it a stack.
//!
//! The initial stack follows QNX's layout (procnto's `setup_tls_ring0`, the
//! SDP's crt `_start`): at the top the thread's `struct
//! _thread_local_storage` (`<sys/storage.h>`, 0x78 bytes, 16-byte aligned)
//! with the 16-byte ELF TLS control block just below it (word 0: 0, word
//! 1: the TLS address), which is where the thread pointer (TPIDR_EL0)
//! points; below that the argument and environment strings, then auxv,
//! envp, argv and argc at the initial stack pointer (System V layout).
//!
//! Position-independent images (ET_DYN) are loaded at fixed bases. A
//! program with PT_INTERP gets its dynamic loader (`ldqnx-64.so.2`, or
//! `ld-elf.so.1`) loaded too and starts there; auxv points the loader at
//! the program (AT_PHDR, AT_ENTRY) and at itself (AT_BASE).
//!
//! A program whose ELF header says `ELFOSABI_FREEBSD` gets a native,
//! FreeBSD process (`Personality::Native`); one with `ELFOSABI_NONE`,
//! which QNX's toolchain emits, a QNX one, started as above. The native
//! start is the stack FreeBSD's `exec_copyout_strings()` builds (`struct ps_strings`
//! at the top, the executable's path, the stack canary, the page sizes,
//! the strings, then argc, argv, envp and FreeBSD's auxiliary vector),
//! no thread-local storage from the kernel (its libc sets its own thread
//! pointer), and the stack pointer in x0, as FreeBSD's arm64
//! `exec_setregs()` passes it (`initial_stack`).
//!
//! Not yet: segments sharing a page with different permissions,
//! address-space randomisation, and PT_TLS for a static program (a
//! dynamic one's TLS is set up by `ldqnx-64.so.2`).

use core::sync::atomic::Ordering;

use crate::arch::mmu;
use crate::arch::uspace::{USER_END, UserProt};
use crate::ker::process::{Personality, Process};
use crate::vm::pmem::PAGE_SIZE;
use crate::vm::{kmem, map, pmem};

const PT_LOAD: u32 = 1;
const PT_INTERP: u32 = 3;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;
const EM_AARCH64: u16 = 183;
const ET_EXEC: u16 = 2;
const ET_DYN: u16 = 3;
/// User stack: pages below an unmapped guard page at the top of user space.
/// The main thread's stack: 512 KB by default, as in QNX (a spawn's
/// `POSIX_SPAWN_SETSTACKMAX` sets it), all allocated at once (QNX 8's
/// memory is superlocked: nothing is faulted in later).
pub const STACK_SIZE: u64 = 128 * PAGE_SIZE;
/// The top of the main thread's stack, and the lowest address the system
/// page may be mapped at: the four pages below the end of the address
/// space are the system page's.
pub const STACK_TOP: u64 = USER_END - 4 * PAGE_SIZE;
/// Where the system page goes if the startup names no usable address.
const SYSPAGE_USER: u64 = STACK_TOP;

/// The system page's address in every process, where it is mapped
/// read-only (`adopt_user_syspage()` chooses it at boot and records it in
/// `system_private.user_syspageptr`, where programs and procnto's tools
/// read it). The kernel used to hand programs its own kernel address of
/// the page, which a server can't read a client's buffer from.
pub fn user_syspage_ptr() -> u64 {
    crate::syspage::get().system_private().user_syspageptr as u64
}

/// At boot: where programs see the system page. As procnto:
///
/// - the startup's address, if it is a user address
///   (`vm_as_create_default()` passes `system_private.user_syspageptr` to
///   `vm_syspage_map()` when it is below the address space's end; read
///   with REA);
/// - otherwise (OsCraft's startup leaves a kernel address there) at the
///   very top of user space, as far below its end as the page's memory is
///   long: the sections, the callouts and the cpupages, which is what
///   `vm_syspage_map()` maps (from the system page to the end of the last
///   cpupage). Measured: 0x7f_ffff_d000 on QEMU's virt with OsCraft's
///   startup, three pages; 0x7f_ffff_c000 with QNX's startup on the
///   Raspberry Pi 4 (review CORE-30). That the distance is this length is
///   inferred from those two and from the length `vm_syspage_map()` maps,
///   not read from procnto's code.
///
/// The stack's top stays where it is: an address the page doesn't fit at
/// above it is replaced by `SYSPAGE_USER`.
pub fn adopt_user_syspage(sp: crate::syspage::Syspage) {
    let va = sp.as_ptr() as u64;
    let p = sp.system_private();
    let given = p.user_syspageptr as u64;
    let span = (p.kern_cpupageptr as u64 + u64::from(sp.header().num_cpu) * u64::from(p.cpupage_spacing))
        .saturating_sub(p.kern_syspageptr as u64)
        .max(u64::from(sp.header().total_size));
    let offset = va % PAGE_SIZE;
    let fits = |at: u64| at >= STACK_TOP && at.saturating_add(u64::from(sp.header().total_size)) <= USER_END && at % PAGE_SIZE == offset;
    let at = if given < USER_END && fits(given) {
        given
    } else {
        let top = USER_END.saturating_sub((span + offset).next_multiple_of(PAGE_SIZE)) + offset;
        if fits(top) { top } else { SYSPAGE_USER + offset }
    };
    // SAFETY: the system page is the kernel's, writable, and nobody has
    // read the field yet (`kmain` calls this first).
    unsafe { (*sp.system_private_ptr()).user_syspageptr = at as usize };
}

/// Map the system page's pages read-only at `user_syspage_ptr()`: physical
/// memory of the process's, not RAM it owns, so an `munmap()` of it frees
/// nothing and a fork shares it.
fn map_syspage(p: &Process) -> Result<(), &'static str> {
    let sp = crate::syspage::get();
    let va = sp.as_ptr() as u64;
    let pa = mmu::translate(va & !(PAGE_SIZE - 1)).ok_or("system page")?;
    let size = (sp.header().total_size as u64 + va % PAGE_SIZE).next_multiple_of(PAGE_SIZE);
    const MAP_PHYS: u32 = 0x1_0000;
    let at = user_syspage_ptr() & !(PAGE_SIZE - 1);
    map::map_phys(p, at, size, map::PROT_READ, map::MAP_SHARED | MAP_PHYS | map::MAP_FIXED, pa)
        .map(|_| ())
        .map_err(|_| "system page")
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// How to start the loaded program's first thread.
pub struct Start {
    pub entry: u64,
    pub sp: u64,
    /// Thread pointer (TPIDR_EL0): QNX's TLS control block, 0 for FreeBSD.
    pub tp: u64,
    /// The first argument register: the thread pointer for a QNX program,
    /// the stack pointer for a FreeBSD one.
    pub x0: u64,
}

const TLS_SIZE: u64 = 0x78;
const TCB_SIZE: u64 = 16;
// auxv types (<sys/auxv.h>)
const AT_NULL: u64 = 0;
const AT_PHDR: u64 = 3;
const AT_PHENT: u64 = 4;
const AT_PHNUM: u64 = 5;
const AT_PAGESZ: u64 = 6;
const AT_BASE: u64 = 7;
const AT_ENTRY: u64 = 9;
const AT_RANDOM: u64 = 43;
const AT_FREE_STACK: u64 = 44;
const AT_INTP_DEVICE: u64 = 45;
const AT_INTP_INODE: u64 = 46;
const AT_EXEFILE: u64 = 47;
// FreeBSD's auxv types (`<sys/elf_common.h>`), where they differ or are its
// own; AT_PHDR..AT_ENTRY are the System V ones above.
const BSD_AT_FLAGS: u64 = 8;
const BSD_AT_EXECPATH: u64 = 15;
const BSD_AT_CANARY: u64 = 16;
const BSD_AT_CANARYLEN: u64 = 17;
const BSD_AT_OSRELDATE: u64 = 18;
const BSD_AT_NCPUS: u64 = 19;
const BSD_AT_PAGESIZES: u64 = 20;
const BSD_AT_PAGESIZESLEN: u64 = 21;
const BSD_AT_STACKPROT: u64 = 23;
const BSD_AT_EHDRFLAGS: u64 = 24;
const BSD_AT_HWCAP: u64 = 25;
const BSD_AT_HWCAP2: u64 = 26;
const BSD_AT_BSDFLAGS: u64 = 27;
const BSD_AT_ARGC: u64 = 28;
const BSD_AT_ARGV: u64 = 29;
const BSD_AT_ENVC: u64 = 30;
const BSD_AT_ENVV: u64 = 31;
const BSD_AT_PS_STRINGS: u64 = 32;
const BSD_AT_USRSTACKBASE: u64 = 35;
const BSD_AT_USRSTACKLIM: u64 = 36;
/// OsCraft's addition to a FreeBSD process's auxiliary vector: its
/// process ID, which libsys's `getpid()` reads (QNX's libc has it in the
/// thread-local block the kernel builds, which a FreeBSD process lacks).
const AT_OSCRAFT_PID: u64 = 1001;
/// `kern.osreldate` a FreeBSD process is told (AT_OSRELDATE): the
/// `__FreeBSD_version` of the userland OsCraft is built against; to be
/// pinned with that source tree.
const BSD_OSRELDATE: u64 = 1_500_000;
/// The canary FreeBSD gives a process (AT_CANARY): 64 bytes.
const BSD_CANARY_LEN: usize = 64;
/// `struct ps_strings` (`<sys/exec.h>`): argv, argc, envp, envc; 32 bytes.
const BSD_PS_STRINGS_SIZE: u64 = 32;

/// The errno of a load failure (its message), as procnto answers the
/// spawn or exec (measured, review PROCMGR-7): ELIBACC for an interpreter
/// that can't be found and for a segment whose file and memory offsets
/// disagree within the page, ENOMEM for a segment the address space has
/// no room for or memory that can't be had, ENOEXEC for a malformed
/// image (two segments on the same pages among them).
pub fn errno_of(msg: &str) -> u32 {
    use crate::ker::errno::{ELIBACC, ENOEXEC, ENOMEM};
    match msg {
        "interpreter not found" | "segment misaligned" => ELIBACC,
        "segment out of range" | "out of memory" | "stack" | "cannot map segment" => ENOMEM,
        _ => ENOEXEC,
    }
}

/// Check `elf` and its interpreter as `load` would, mapping nothing: an
/// exec checks before it tears the old image down, so that a load that
/// can't succeed fails the exec and leaves the process as it was
/// (procnto, measured, review PROCMGR-7). The memory the segments and
/// the stack take is checked against what is free (superlocked memory
/// is backed at load time): ENOMEM for an image of 8 GiB, as procnto.
pub fn check(elf: &[u8], stack_size: u64) -> Result<(), &'static str> {
    let fs = super::imagefs::get().ok_or("no image filesystem")?;
    let (interp, mut pages) = validate(elf, PROGRAM_BASE)?;
    if let Some(interp) = interp {
        let (file, _) = interpreter(fs, interp)?;
        let (nested, more) = validate(file.data, INTERP_BASE)?;
        if nested.is_some() {
            return Err("interpreter has an interpreter");
        }
        pages = pages.saturating_add(more);
    }
    pages = pages.saturating_add(stack_size.div_ceil(PAGE_SIZE).max(1));
    if !pmem::can_spare(pages) {
        return Err("out of memory");
    }
    Ok(())
}

/// The interpreter a program names, and its path: found as `open()`
/// finds a file, through the image's links (procnto's `elf64_load` opens
/// it with an ordinary connect; measured, #326), and a regular file.
fn interpreter(
    fs: &super::imagefs::ImageFs,
    name: &str,
) -> Result<(super::imagefs::Entry, alloc::string::String), &'static str> {
    use super::imagefs::{S_IFMT, S_IFREG};
    fs.follow(name)
        .filter(|(e, _)| e.mode & S_IFMT == S_IFREG)
        .ok_or("interpreter not found")
}

/// Load `elf` (from `path`) into `p` and build its initial stack.
pub fn load(
    p: &Process,
    elf: &[u8],
    path: &str,
    argv: &[&[u8]],
    envp: &[&[u8]],
    stack_size: u64,
) -> Result<Start, &'static str> {
    let stack_pages = stack_size.div_ceil(PAGE_SIZE).max(1);
    let name = map::object_name(path.trim_start_matches('/'));
    let prog = map_image(p, elf, PROGRAM_BASE, name)?;
    // The header was checked by `map_image`; byte 7 is EI_OSABI.
    p.set_personality(Personality::from_osabi(elf[7]));
    *p.name.lock() = path.trim_start_matches('/').into();
    crate::ker::trace::process_named(p);
    let mut entry = prog.entry;
    let mut interp_base = 0;
    let mut interp_file = None;
    if let Some(interp) = prog.interp {
        let fs = super::imagefs::get().ok_or("no image filesystem")?;
        let (file, real) = interpreter(fs, interp)?;
        interp_file = Some((super::resmgr::IFS_DEV as u64, file.ino as u64));
        let ld = map_image(p, file.data, INTERP_BASE, map::object_name(&real))?;
        if ld.interp.is_some() {
            return Err("interpreter has an interpreter");
        }
        entry = ld.entry;
        interp_base = INTERP_BASE;
    }
    // The stack's region is recorded before its pages are mapped: a
    // failure part-way leaves pages the process's teardown finds and
    // frees (review PROCMGR-14: they leaked).
    let base = STACK_TOP - stack_pages * PAGE_SIZE;
    {
        let mut mem = p.mem.lock();
        // procnto marks the main thread's stack (and its guard) with
        // `_MAP_RESERVE11` too (measured).
        const MAIN_STACK: u32 = map::MAP_PRIVATE | map::MAP_ANON | map::MAP_STACK | 0x8000_0000;
        mem.record(base, STACK_TOP, map::PROT_READ | map::PROT_WRITE, MAIN_STACK, None);
        // A guard page below, never mapped (as procnto's: an overflow
        // faults, SEGV_MAPERR, and the stack's size counts it).
        mem.record(base - PAGE_SIZE, base, map::PROT_NONE, MAIN_STACK, None);
    }
    for i in 1..=stack_pages {
        let pa = pmem::alloc_user_page().ok_or("out of memory")?;
        kmem::with_frame(pa, |page| page.fill(0));
        let mapped = p.aspace().map(
            STACK_TOP - i * PAGE_SIZE,
            pa,
            UserProt {
                read: true,
                write: true,
                exec: false,
            },
        );
        if mapped.is_err() {
            pmem::free_page(pa);
            return Err("stack");
        }
    }
    // The system page, read-only (CORE-30), once the stack is in place.
    map_syspage(p)?;
    let aux = Aux {
        phdr: prog.phdr,
        phnum: prog.phnum,
        entry: prog.entry,
        base: interp_base,
        interp: interp_file,
    };
    let stack_size = stack_pages * PAGE_SIZE;
    let start = match p.personality() {
        Personality::Qnx => initial_stack_qnx(p, entry, aux, path, argv, envp, stack_size)?,
        Personality::Native => initial_stack(p, entry, aux, path, argv, envp, stack_size)?,
    };
    p.base_address.store(prog.base, Ordering::Relaxed);
    p.initial_stack.store(start.sp, Ordering::Relaxed);
    Ok(start)
}

/// Load bases for position-independent images (ET_DYN): the program, and
/// the dynamic loader named by its PT_INTERP.
const PROGRAM_BASE: u64 = 1 << 32;
const INTERP_BASE: u64 = 4 << 32;

struct Image<'a> {
    /// The lowest address of its segments.
    base: u64,
    entry: u64,
    phdr: u64,
    phnum: u64,
    interp: Option<&'a str>,
}

/// The checks `map_image` makes of an image's shape, without mapping
/// it; returns its interpreter's name, if it has one, and the pages its
/// PT_LOAD segments take. The messages here and in `segment_shape`,
/// `header`, `interp_name` and `load` are matched by `errno_of`: a new
/// one gets its errno there. The order is procnto's, measured: a segment
/// past the end of the address space is ENOMEM whatever else is wrong
/// with it, a misaligned one ELIBACC even where it also lands on
/// another's pages, and two well-formed segments on the same pages
/// ENOEXEC.
fn validate(elf: &[u8], base: u64) -> Result<(Option<&str>, u64), &'static str> {
    let (bias, phoff, phnum) = header(elf, base)?;
    let mut interp = None;
    let mut pages = 0u64;
    let mut loads: alloc::vec::Vec<(u64, u64)> = alloc::vec::Vec::new();
    for i in 0..phnum {
        let ph = phoff + i * 56;
        let (off, filesz, memsz) = (u64_at(elf, ph + 8), u64_at(elf, ph + 32), u64_at(elf, ph + 40));
        let vaddr = u64_at(elf, ph + 16).checked_add(bias).ok_or("segment out of range")?;
        segment_shape(elf, off, filesz, memsz, vaddr)?;
        match u32_at(elf, ph) {
            PT_INTERP => interp = Some(interp_name(elf, off, filesz)?),
            PT_LOAD if memsz > 0 => {
                let page = vaddr & !(PAGE_SIZE - 1);
                let end = (vaddr + memsz).next_multiple_of(PAGE_SIZE);
                if loads.iter().any(|&(s, e)| page < e && s < end) {
                    return Err("segments overlap");
                }
                loads.push((page, end));
                pages = pages.saturating_add((end - page) / PAGE_SIZE);
            }
            _ => {}
        }
    }
    Ok((interp, pages))
}

/// The ELF header's checks: the load bias, and where the program
/// headers are and how many.
fn header(elf: &[u8], base: u64) -> Result<(u64, usize, usize), &'static str> {
    if elf.len() < 64 || &elf[..4] != b"\x7fELF" || elf[4] != 2 || elf[5] != 1 {
        return Err("not a 64-bit little-endian ELF file");
    }
    let bias = match u16_at(elf, 16) {
        ET_EXEC => 0,
        ET_DYN => base,
        _ => return Err("not an executable or shared object"),
    };
    if u16_at(elf, 18) != EM_AARCH64 {
        return Err("not an AArch64 image");
    }
    if u16_at(elf, 54) != 56 {
        return Err("program header size is not 56");
    }
    let (phoff, phnum) = (u64_at(elf, 32) as usize, u16_at(elf, 56) as usize);
    if phoff.checked_add(phnum * 56).is_none_or(|end| end > elf.len()) {
        return Err("program headers out of range");
    }
    Ok((bias, phoff, phnum))
}

/// A segment's shape as the ELF format promises it: within the file, no
/// larger in the file than in memory, file and memory offsets alike
/// within a page. A segment's memory range is checked first by the
/// caller, where it matters: one past the end of the address space is
/// ENOMEM on procnto whatever else is wrong with it (measured).
fn segment_shape(elf: &[u8], off: u64, filesz: u64, memsz: u64, vaddr: u64) -> Result<(), &'static str> {
    if off.checked_add(filesz).is_none_or(|end| end > elf.len() as u64) {
        return Err("segment past the end of the file");
    }
    if filesz > memsz {
        return Err("segment larger in the file than in memory");
    }
    if vaddr.checked_add(memsz).is_none_or(|e| e > USER_END) {
        return Err("segment out of range");
    }
    if filesz != 0 && off % PAGE_SIZE != vaddr % PAGE_SIZE {
        return Err("segment misaligned");
    }
    Ok(())
}

/// The PT_INTERP segment's string.
fn interp_name(elf: &[u8], off: u64, filesz: u64) -> Result<&str, &'static str> {
    let name = &elf[off as usize..(off + filesz) as usize];
    let name = name.split(|&c| c == 0).next().unwrap_or(b"");
    core::str::from_utf8(name).map_err(|_| "bad interpreter name")
}

/// Map the PT_LOAD segments of `elf` into `p`, at `base` if it is
/// position-independent (ET_DYN).
fn map_image<'a>(
    p: &Process,
    elf: &'a [u8],
    base: u64,
    name: &'static str,
) -> Result<Image<'a>, &'static str> {
    validate(elf, base)?;
    if elf.len() < 64 || &elf[..4] != b"\x7fELF" || elf[4] != 2 || elf[5] != 1 {
        return Err("not a 64-bit little-endian ELF file");
    }
    let bias = match u16_at(elf, 16) {
        ET_EXEC => 0,
        ET_DYN => base,
        _ => return Err("not an executable or shared object"),
    };
    if u16_at(elf, 18) != EM_AARCH64 {
        return Err("not an AArch64 image");
    }
    // Every header field is the file's (anyone can write one in /tmp):
    // checked arithmetic throughout, and the shape the ELF format
    // promises (a 56-byte program header, a segment no larger in the file
    // than in memory, file and memory offsets alike within a page) is
    // required rather than assumed.
    if u16_at(elf, 54) != 56 {
        return Err("program header size is not 56");
    }
    let (phoff, phnum) = (u64_at(elf, 32) as usize, u16_at(elf, 56) as usize);
    if phoff.checked_add(phnum * 56).is_none_or(|end| end > elf.len()) {
        return Err("program headers out of range");
    }
    let mut interp = None;
    let mut lowest = u64::MAX;
    for i in 0..phnum {
        let ph = phoff + i * 56;
        let p_type = u32_at(elf, ph);
        let flags = u32_at(elf, ph + 4);
        let (off, filesz, memsz) = (u64_at(elf, ph + 8), u64_at(elf, ph + 32), u64_at(elf, ph + 40));
        let vaddr = u64_at(elf, ph + 16).checked_add(bias).ok_or("segment out of range")?;
        segment_shape(elf, off, filesz, memsz, vaddr)?;
        if p_type == PT_INTERP {
            interp = Some(interp_name(elf, off, filesz)?);
            continue;
        }
        if p_type != PT_LOAD {
            continue;
        }
        // The segment's end in memory, checked once: every sum below
        // (`vaddr + filesz` too, as `filesz <= memsz`) stays below it, and
        // `USER_END` is far from the top of the address arithmetic.
        let seg_end = vaddr
            .checked_add(memsz)
            .filter(|&e| e <= USER_END)
            .ok_or("segment out of range")?;
        let prot = UserProt {
            read: flags & PF_R != 0,
            write: flags & PF_W != 0,
            exec: flags & PF_X != 0,
        };
        let mut page = vaddr & !(PAGE_SIZE - 1);
        lowest = lowest.min(page);
        let qnx_prot = [
            (PF_R, map::PROT_READ),
            (PF_W, map::PROT_WRITE),
            (PF_X, map::PROT_EXEC),
        ]
        .iter()
        .filter(|(pf, _)| flags & pf != 0)
        .fold(0, |acc, (_, prot)| acc | prot);
        // Code and read-only data, all from the file and in pages no other
        // segment uses: mapped in place from the image filesystem, shared
        // by every process that runs it (as QNX runs programs from it).
        let end = seg_end.next_multiple_of(PAGE_SIZE);
        let alone = (0..phnum).filter(|&j| j != i).all(|j| {
            let q = phoff + j * 56;
            let (qv, qm) = (u64_at(elf, q + 16).saturating_add(bias), u64_at(elf, q + 40));
            u32_at(elf, q) != PT_LOAD || qv.saturating_add(qm) <= page || qv >= end
        });
        let in_place = !prot.write
            && filesz == memsz
            && off % PAGE_SIZE == vaddr % PAGE_SIZE
            && alone
            && p.aspace().translate(page, false).is_none();
        if in_place
            && let Some(pa) = super::imagefs::phys_of(&elf[(off - off % PAGE_SIZE) as usize..])
            && pa.is_multiple_of(PAGE_SIZE)
        {
            // Code is mapped shared, as procnto's loader maps it (measured).
            let share = if prot.exec { map::MAP_SHARED } else { map::MAP_PRIVATE };
            map::map_image(
                p,
                page,
                end - page,
                qnx_prot,
                share | map::MAP_ELF | map::MAP_FIXED,
                0,
                pa,
                Some(name),
            )
            .map_err(|_| "cannot map segment")?;
            p.mem.lock().set_offset(page, off - (vaddr - page));
            continue;
        }
        // The file's pages, then the .bss beyond them as anonymous memory
        // (procnto's regions, measured).
        let file_end = (vaddr + filesz).next_multiple_of(PAGE_SIZE).max(page);
        let mut mem = p.mem.lock();
        mem.record(page, file_end, qnx_prot, map::MAP_PRIVATE | map::MAP_ELF | map::MAP_FIXED, Some(name));
        mem.set_offset(page, off - (vaddr - page));
        mem.record(
            file_end,
            end,
            qnx_prot,
            map::MAP_PRIVATE | map::MAP_ANON | map::MAP_ELF | map::MAP_FIXED,
            None,
        );
        drop(mem);
        while page < seg_end {
            let (pa, fresh) = match p.aspace().translate(page, false) {
                Some(pa) => (pa, false), // shared with the previous segment
                None => {
                    let pa = pmem::alloc_user_page().ok_or("out of memory")?;
                    p.aspace()
                        .map(page, pa, prot)
                        .map_err(|_| "cannot map segment")?;
                    (pa, true)
                }
            };
            // File bytes that land in this page.
            let (lo, hi) = (page.max(vaddr), (page + PAGE_SIZE).min(vaddr + filesz));
            kmem::with_frame(pa, |frame| {
                if fresh {
                    frame.fill(0); // .bss and padding
                }
                if lo < hi {
                    let src = &elf[(off + lo - vaddr) as usize..(off + hi - vaddr) as usize];
                    let at = (lo - page) as usize;
                    frame[at..at + src.len()].copy_from_slice(src);
                }
                if prot.exec {
                    mmu::sync_icache(frame.as_ptr() as u64, PAGE_SIZE);
                }
            });
            page += PAGE_SIZE;
        }
    }
    // Program headers in memory: inside the segment that maps file offset 0.
    let phdr = (0..phnum)
        .map(|i| phoff + i * 56)
        .find(|&ph| u32_at(elf, ph) == PT_LOAD && u64_at(elf, ph + 8) == 0)
        .map_or(0, |ph| u64_at(elf, ph + 16) + bias + phoff as u64);
    Ok(Image {
        base: lowest,
        entry: u64_at(elf, 24) + bias,
        phdr,
        phnum: phnum as u64,
        interp,
    })
}

/// What the auxiliary vector tells the program (or its dynamic loader).
struct Aux {
    phdr: u64,
    phnum: u64,
    entry: u64,
    /// Load base of the dynamic loader (0 for static programs).
    base: u64,
    /// The dynamic loader's file: its `st_dev` and `st_ino`.
    interp: Option<(u64, u64)>,
}

/// Build the TLS, strings, auxv, envp, argv and argc at the top of the stack.
fn initial_stack_qnx(
    p: &Process,
    entry: u64,
    aux: Aux,
    path: &str,
    argv: &[&[u8]],
    envp: &[&[u8]],
    stack_size: u64,
) -> Result<Start, &'static str> {
    use alloc::vec::Vec;
    let tls = (STACK_TOP - TLS_SIZE) & !15;
    let tp = tls - TCB_SIZE;
    // Strings, placed downwards from the thread pointer.
    let mut strings: Vec<u8> = Vec::new();
    let mut add = |s: &[u8]| {
        let off = strings.len();
        strings.extend_from_slice(s);
        strings.push(0);
        off
    };
    let exe_off = add(path.as_bytes());
    let arg_offs: Vec<usize> = argv.iter().map(|a| add(a)).collect();
    let env_offs: Vec<usize> = envp.iter().map(|e| add(e)).collect();
    let random_off = strings.len();
    let seed = crate::arch::timer::counter();
    strings.extend_from_slice(&seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).to_le_bytes());
    strings.extend_from_slice(&(seed ^ 0x5bd1_e995_c2b2_ae35).rotate_left(17).to_le_bytes());
    let strings_at = (tp - strings.len() as u64) & !15;
    let s = |off: usize| strings_at + off as u64;

    let mut words: Vec<u64> = Vec::new();
    words.push(argv.len() as u64);
    words.extend(arg_offs.iter().map(|&o| s(o)));
    words.push(0);
    words.extend(env_offs.iter().map(|&o| s(o)));
    words.push(0);
    // procnto's auxiliary vector, in its order (measured, review
    // PROCMGR-13): no AT_UID/AT_EUID/AT_GID/AT_EGID (the dynamic loader
    // asks the kernel), the interpreter's device and inode, and the
    // stack's room, its size less one page.
    let mut aux_words = alloc::vec![
        (AT_PHDR, aux.phdr),
        (AT_PHENT, 56),
        (AT_PHNUM, aux.phnum),
        (AT_ENTRY, aux.entry),
        (AT_PAGESZ, PAGE_SIZE),
        (AT_BASE, aux.base),
    ];
    if let Some((dev, ino)) = aux.interp {
        aux_words.extend([(AT_INTP_DEVICE, dev), (AT_INTP_INODE, ino)]);
    }
    aux_words.extend([
        (AT_EXEFILE, s(exe_off)),
        (AT_RANDOM, s(random_off)),
        (AT_FREE_STACK, stack_size - PAGE_SIZE),
        (AT_NULL, 0),
    ]);
    for (t, v) in aux_words {
        words.extend([t, v]);
    }
    let sp = (strings_at - words.len() as u64 * 8) & !15;
    if sp < STACK_TOP - stack_size {
        return Err("arguments too large for the stack");
    }
    let bytes = |w: &[u64]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();

    // ANCHOR: qnx_thread_block
    let mut tlsb = [0u8; TLS_SIZE as usize];
    let mut put = |off: usize, v: &[u8]| tlsb[off..off + v.len()].copy_from_slice(v);
    put(0x00, &stack_size.to_le_bytes()); // __stacksize
    put(0x10, &(tls + 0x18).to_le_bytes()); // __errptr = &__errval
    put(0x20, &p.pid.to_le_bytes()); // __pid
    put(0x24, &1i32.to_le_bytes()); // __tid: the main thread is 1
    // __owner, the value libc stores in the mutexes the thread locks
    // (procnto's thread_bootstrap: pid in the high half, tid in the low).
    put(0x28, &(((p.pid as u32 & 0x7fff) << 16) | 1).to_le_bytes());
    put(0x30, &(STACK_TOP - stack_size).to_le_bytes()); // __stackaddr
    let tcb = bytes(&[0, tls]);
    // ANCHOR_END: qnx_thread_block

    // SAFETY: `p` is borrowed for the whole call.
    let user = unsafe { crate::ker::umem::Space::user(p) };
    let daif = crate::arch::irq_save();
    let r = crate::ker::umem::write(user, tls, &tlsb)
        .and_then(|_| crate::ker::umem::write(user, tp, &tcb))
        .and_then(|_| crate::ker::umem::write(user, strings_at, &strings))
        .and_then(|_| crate::ker::umem::write(user, sp, &bytes(&words)));
    crate::arch::irq_restore(daif);
    r.map_err(|_| "cannot write the initial stack")?;
    Ok(Start { entry, sp, tp, x0: tp })
}

/// Build a FreeBSD process's initial stack (its `exec_copyout_strings()`
/// and arm64 `exec_setregs()`, `sys/kern/kern_exec.c` and
/// `sys/arm64/arm64/machdep.c` of FreeBSD 15): from the top, `struct
/// ps_strings`, the executable's path, the canary, the page-size array,
/// the argument and environment strings, then, 16-byte aligned, argc, the
/// argv and envp pointers and the auxiliary vector in the order
/// `__elfN(freebsd_copyout_auxargs)` writes it. No TLS: FreeBSD's libc
/// allocates it and sets the thread pointer itself. The entry gets the
/// stack pointer in x0.
///
/// AT_HWCAP and AT_HWCAP2 are 0 for now: the CPU's ID registers are not
/// read for them yet, so libc takes no ifunc fast paths.
fn initial_stack(
    p: &Process,
    entry: u64,
    aux: Aux,
    path: &str,
    argv: &[&[u8]],
    envp: &[&[u8]],
    stack_size: u64,
) -> Result<Start, &'static str> {
    use alloc::vec::Vec;
    let bytes = |w: &[u64]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let ps_strings = STACK_TOP - BSD_PS_STRINGS_SIZE;
    // Below ps_strings: the data the auxv points at, then the strings.
    let mut data: Vec<u8> = Vec::new();
    let mut add = |s: &[u8]| {
        let off = data.len();
        data.extend_from_slice(s);
        off
    };
    let exec_off = add(path.as_bytes());
    // NUL, then padding: the canary and the page sizes are
    // pointer-aligned, as FreeBSD rounds each item down to `sizeof(void *)`.
    add(&[0; 8][..8 - path.len() % 8]);
    let canary_off = data.len();
    let mut seed = crate::arch::timer::counter();
    for _ in 0..BSD_CANARY_LEN / 8 {
        seed = (seed ^ 0x5bd1_e995_c2b2_ae35).wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17);
        data.extend_from_slice(&seed.to_le_bytes());
    }
    let pagesizes_off = data.len();
    data.extend_from_slice(&PAGE_SIZE.to_le_bytes());
    let mut add_str = |s: &[u8]| {
        let off = data.len();
        data.extend_from_slice(s);
        data.push(0);
        off
    };
    let arg_offs: Vec<usize> = argv.iter().map(|a| add_str(a)).collect();
    let env_offs: Vec<usize> = envp.iter().map(|e| add_str(e)).collect();
    let data_at = (ps_strings - data.len() as u64) & !15;
    let d = |off: usize| data_at + off as u64;

    let mut words: Vec<u64> = Vec::new();
    words.push(argv.len() as u64);
    let argv_index = words.len();
    words.extend(arg_offs.iter().map(|&o| d(o)));
    words.push(0);
    let envp_index = words.len();
    words.extend(env_offs.iter().map(|&o| d(o)));
    words.push(0);
    let auxv_index = words.len();
    let ncpus = crate::syspage::get().header().num_cpu as u64;
    // ANCHOR: native_auxv
    let aux_words = [
        (AT_PHDR, aux.phdr),
        (AT_PHENT, 56),
        (AT_PHNUM, aux.phnum),
        (AT_PAGESZ, PAGE_SIZE),
        (BSD_AT_FLAGS, 0),
        (AT_ENTRY, aux.entry),
        (AT_BASE, aux.base),
        (BSD_AT_EHDRFLAGS, 0),
        (BSD_AT_EXECPATH, d(exec_off)),
        (BSD_AT_OSRELDATE, BSD_OSRELDATE),
        (BSD_AT_CANARY, d(canary_off)),
        (BSD_AT_CANARYLEN, BSD_CANARY_LEN as u64),
        (BSD_AT_NCPUS, ncpus),
        (BSD_AT_PAGESIZES, d(pagesizes_off)),
        (BSD_AT_PAGESIZESLEN, 8),
        (BSD_AT_STACKPROT, 3), // FreeBSD's PROT_READ | PROT_WRITE
        (BSD_AT_HWCAP, 0),
        (BSD_AT_HWCAP2, 0),
        (BSD_AT_BSDFLAGS, 0),
        (BSD_AT_ARGC, argv.len() as u64),
        (BSD_AT_ARGV, 0), // patched below: the address of the argv pointers
        (BSD_AT_ENVC, envp.len() as u64),
        (BSD_AT_ENVV, 0), // patched below
        (BSD_AT_PS_STRINGS, ps_strings),
        (BSD_AT_USRSTACKBASE, STACK_TOP),
        (BSD_AT_USRSTACKLIM, stack_size),
        (AT_OSCRAFT_PID, p.pid as u64),
        (AT_NULL, 0),
    ];
    // ANCHOR_END: native_auxv
    for (t, v) in aux_words {
        words.extend([t, v]);
    }
    let sp = (data_at - words.len() as u64 * 8) & !15;
    if sp < STACK_TOP - stack_size {
        return Err("arguments too large for the stack");
    }
    let (argv_at, envp_at) = (sp + argv_index as u64 * 8, sp + envp_index as u64 * 8);
    for i in (auxv_index..words.len()).step_by(2) {
        match words[i] {
            BSD_AT_ARGV => words[i + 1] = argv_at,
            BSD_AT_ENVV => words[i + 1] = envp_at,
            _ => {}
        }
    }
    let mut psb = [0u8; BSD_PS_STRINGS_SIZE as usize];
    psb[0..8].copy_from_slice(&argv_at.to_le_bytes());
    psb[8..12].copy_from_slice(&(argv.len() as u32).to_le_bytes());
    psb[16..24].copy_from_slice(&envp_at.to_le_bytes());
    psb[24..28].copy_from_slice(&(envp.len() as u32).to_le_bytes());

    // SAFETY: `p` is borrowed for the whole call.
    let user = unsafe { crate::ker::umem::Space::user(p) };
    let daif = crate::arch::irq_save();
    let r = crate::ker::umem::write(user, ps_strings, &psb)
        .and_then(|_| crate::ker::umem::write(user, data_at, &data))
        .and_then(|_| crate::ker::umem::write(user, sp, &bytes(&words)));
    crate::arch::irq_restore(daif);
    r.map_err(|_| "cannot write the initial stack")?;
    Ok(Start { entry, sp, tp: 0, x0: sp })
}

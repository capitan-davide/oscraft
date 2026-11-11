//! A kernel crash handed to a kernel debugger or dumper (QNX's `kdumper`).
//!
//! Contract, from `<sys/kdebug.h>`, `<sys/kdump.h>` and QNX's kdumper (a
//! bootstrap executable that runs before the kernel, see the image's boot
//! section):
//! - At boot the dumper registers a `struct kdebug_callback` in the system
//!   page (`system_private.kdebug_call`); its `fault_entry(entry, sigcode,
//!   regs)` is what a fatal fault calls.
//! - `sigcode` is the signal number, its code `<< 8` and the fault number
//!   (`<sys/fault.h>`) `<< 16`, with `SIGCODE_KERNEL | SIGCODE_FATAL` for a
//!   kernel crash (kdumper acts on those flags); `regs` is the CPU's
//!   `AARCH64_CPU_REGISTERS` (x0..x30, sp, elr, pstate). `TPIDR_EL1` holds
//!   the CPU number, as in procnto (the kernel's per-CPU pointer is lost:
//!   a crash does not come back).
//! - The dumper then writes an ELF dump of memory, the system page and the
//!   registers (by default uuencoded to the debug console; QNX's kdserver
//!   serves it to gdb) and resets the machine through the reboot callout.
//!
//! - The kernel publishes its `struct kdebug_info` in the system page
//!   (`system_private.kdebug_info`): its `request()` answers the dumper's
//!   `KDREQ_DUMP_SWEEP` (the next physical range to dump, starting from
//!   `paddr` -1, -1 at the end; `do_filter` for the kernel's own memory
//!   only: its heap, stacks and windows, the kernel's mapped addresses,
//!   and its translation tables, which kdserver walks from the dump's
//!   TTBR1 to read kernel addresses),
//!   `KDREQ_PARKIT` and `KDREQ_SHUTDOWN_REPORT`.
//!
//! Not yet: process and thread tables in `kdbg_private` (the dump has no
//! process names or page tables), parking the other CPUs, the kprintf ring
//! buffer.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::arch::exception::TrapFrame;

/// `SIGCODE_FATAL`, `SIGCODE_KERNEL` (QNX's kernel macros).
const SIGCODE_FATAL: u32 = 0x0100_0000;
const SIGCODE_KERNEL: u32 = 0x0400_0000;

/// Signals and faults a crash is reported as.
pub const SIGILL: u32 = 4;
pub const SIGABRT: u32 = 6;
pub const SIGSEGV: u32 = 11;
pub const FLTILL: u32 = 1;
pub const FLTPAGE: u32 = 11;

/// `AARCH64_CPU_REGISTERS`: x0..x30, sp, elr, pstate.
#[repr(C, align(16))]
pub struct CpuRegisters {
    pub gpr: [u64; 32],
    pub elr: u64,
    pub pstate: u64,
}

type FaultEntry = unsafe extern "C" fn(*mut u8, u64, *mut CpuRegisters) -> u32;

/// The `sigcode` of a kernel crash.
pub fn sigcode(signo: u32, code: u32, fault: u32) -> u32 {
    signo | (code << 8) | (fault << 16) | SIGCODE_KERNEL | SIGCODE_FATAL
}

// ANCHOR: kdebug_crash
/// Report a kernel crash to the registered debugger or dumper, with the
/// registers of `frame` (an exception's) or else the caller's. Returns
/// if there is none (or it lets the kernel go on); once only.
pub fn crash(frame: Option<&TrapFrame>, sigcode: u32) {
    static ONCE: AtomicBool = AtomicBool::new(false);
    if ONCE.swap(true, Ordering::AcqRel) {
        return;
    }
    let call = crate::syspage::get().system_private().kdebug_call;
    if call == 0 {
        return;
    }
    // struct kdebug_callback: version, length, reserved1, fault_entry.
    // SAFETY: the dumper registered it in the system page; it stays.
    let fault_entry = unsafe { *((call + 16) as *const usize) };
    if fault_entry == 0 {
        return;
    }
    let mut regs = CpuRegisters {
        gpr: [0; 32],
        elr: 0,
        pstate: 0,
    };
    match frame {
        Some(f) => {
            regs.gpr[..31].copy_from_slice(&f.x);
            // EL1's stack pointer: above the frame the exception pushed.
            regs.gpr[31] = f as *const TrapFrame as u64 + core::mem::size_of::<TrapFrame>() as u64;
            regs.elr = f.elr;
            regs.pstate = f.spsr;
        }
        None => crate::arch::exception::capture_registers(&mut regs.gpr, &mut regs.elr, &mut regs.pstate),
    }
    let cpu = crate::percpu::this().cpu as u64;
    // SAFETY: the dumper finds the CPU's number in TPIDR_EL1. From here
    // on `percpu::this()` is not to be used.
    unsafe { core::arch::asm!("msr tpidr_el1, {}", in(reg) cpu, options(nomem, nostack)) };
    // SAFETY: the address of the dumper's C-ABI entry.
    let f: FaultEntry = unsafe { core::mem::transmute(fault_entry) };
    // SAFETY: the entry's arguments; it may not return (it resets).
    unsafe { f(core::ptr::null_mut(), sigcode as u64, &mut regs) };
}
// ANCHOR_END: kdebug_crash

/// `struct kdebug_info` (64-bit).
#[repr(C)]
struct KdebugInfo {
    proc_version: u8,
    debug_version: u8,
    flags: u16,
    unused1: [usize; 3],
    request: unsafe extern "C" fn(*mut u8) -> i32,
    kdbg_private: *const KdebugPrivate,
    unused2: usize,
    timestamp: *const u8,
    unused3: usize,
    kdump_private: usize,
    reserved: [usize; 5],
}

/// `struct kdebug_private` (64-bit): no thread or process tables yet.
#[repr(C)]
struct KdebugPrivate {
    flags: u32,
    actives: usize,
    aspaces: usize,
    irqstack: usize,
    process_vector: usize,
    vector_ver: u16,
    offsets: [u16; 11],
    percpu: usize,
    percpu_stride: u16,
    percpu_active_off: u16,
    muon_cpus: usize,
    mcpu_stride: u16,
    mcpu_active_off: u16,
    mthr_nto_thread_off: u16,
}

// SAFETY: read-only after boot, and only by a crash's dumper.
unsafe impl Sync for KdebugInfo {}
// SAFETY: as above.
unsafe impl Sync for KdebugPrivate {}

/// `KDEBUG_PROC_CURRENT`, `KDEBUG_VECTOR_VER`.
const KDEBUG_PROC_CURRENT: u8 = 3;
const KDEBUG_VECTOR_VER: u16 = 2;
const KDREQ_PARKIT: u32 = 2;
const KDREQ_DUMP_SWEEP: u32 = 5;
const KDREQ_SHUTDOWN_REPORT: u32 = 6;

static PRIVATE: KdebugPrivate = KdebugPrivate {
    flags: 0,
    actives: 0,
    aspaces: 0,
    irqstack: 0,
    process_vector: 0,
    vector_ver: KDEBUG_VECTOR_VER,
    offsets: [0; 11],
    percpu: 0,
    percpu_stride: 0,
    percpu_active_off: 0,
    muon_cpus: 0,
    mcpu_stride: 0,
    mcpu_active_off: 0,
    mthr_nto_thread_off: 0,
};

static TIMESTAMP: &[u8] = concat!("OsCraft ", env!("CARGO_PKG_VERSION"), "\0").as_bytes();

static INFO: KdebugInfo = KdebugInfo {
    proc_version: KDEBUG_PROC_CURRENT,
    debug_version: 0,
    flags: 0,
    unused1: [0; 3],
    request,
    kdbg_private: &PRIVATE,
    unused2: 0,
    timestamp: TIMESTAMP.as_ptr(),
    unused3: 0,
    kdump_private: 0,
    reserved: [0; 5],
};

/// Where the dumper maps the memory it reads: a page per CPU from 2 GB
/// into the kernel's address space (procnto's layout), with tables
/// there (the first 2 MiB of `kmem`'s addresses, which it never hands
/// out).
const DUMP_MAP_VA: u64 = crate::arch::mmu::KERN_BASE + (2 << 30);

/// Publish the kernel's `kdebug_info` in the system page (at boot).
#[expect(
    clippy::multiple_unsafe_ops_per_block,
    reason = "each `(&raw mut (*q).field)` projection and its volatile access are one access to one field"
)]
pub fn init() {
    let cpus = crate::syspage::get().header().num_cpu as u64;
    for cpu in 0..cpus {
        let _ = crate::arch::mmu::ensure_tables(DUMP_MAP_VA + cpu * crate::vm::pmem::PAGE_SIZE);
    }
    let p = crate::syspage::get().system_private_ptr();
    // SAFETY: a field of the system page's private section.
    unsafe { (&raw mut (*p).kdebug_info).write_volatile(&INFO as *const KdebugInfo as usize) };
}

/// Where a filtered memory sweep goes on: the next kernel address to look
/// at.
static SWEEP_VA: AtomicU64 = AtomicU64::new(0);
/// For an unfiltered sweep: the index of the next system RAM range.
static SWEEP_RANGE: AtomicU64 = AtomicU64::new(0);
/// The frame a filtered sweep's last range started at: the next kernel
/// pages mapping the same one (the per-CPU windows) are not dumped again.
static SWEEP_LAST: AtomicU64 = AtomicU64::new(u64::MAX);
/// After the kernel's addresses: where the walk of its translation tables
/// goes on (`mmu::next_table_frame`).
static SWEEP_TABLE: AtomicU64 = AtomicU64::new(0);

/// The kernel's `kdebug_info.request()` (`union kd_request`).
unsafe extern "C" fn request(r: *mut u8) -> i32 {
    // SAFETY: the dumper passes a `union kd_request`, whose first word
    // is the request.
    let req = unsafe { (r as *const u32).read() };
    match req {
        KDREQ_DUMP_SWEEP => {
            // SAFETY: `dump_sweep`'s `do_filter`, after the request.
            let do_filter = unsafe { (r.wrapping_add(4) as *const i32).read() } != 0;
            let paddr = r.wrapping_add(8) as *mut u64;
            let size = r.wrapping_add(16) as *mut u64;
            // SAFETY: `dump_sweep`'s `paddr`, in and out.
            let (p, s) = sweep(unsafe { paddr.read() } == u64::MAX, do_filter);
            // SAFETY: as above.
            unsafe { paddr.write(p) };
            // SAFETY: `dump_sweep`'s `size`, out.
            unsafe { size.write(s) };
        }
        KDREQ_SHUTDOWN_REPORT => {
            // SAFETY: `shutdown_report`'s `sigcode`, after the request.
            let sigcode = unsafe { (r.wrapping_add(4) as *const u32).read() };
            crate::kprintln!("kernel crash: sigcode {sigcode:#x}, dumping");
        }
        KDREQ_PARKIT => {}
        _ => return -1,
    }
    0
}

/// The next physical range of a memory sweep (`u64::MAX` at the end):
/// the kernel's own pages (its mapped addresses, coalesced where the
/// frames follow each other, then its translation tables), or all of
/// system RAM.
fn sweep(start: bool, filter: bool) -> (u64, u64) {
    const PAGE: u64 = crate::vm::pmem::PAGE_SIZE;
    if !filter {
        let i = if start { 0 } else { SWEEP_RANGE.load(Ordering::Relaxed) };
        let sp = crate::syspage::get();
        let r = sp.asinfo().filter(|a| sp.string(a.name) == "sysram").nth(i as usize);
        SWEEP_RANGE.store(i + 1, Ordering::Relaxed);
        return r.map_or((u64::MAX, 0), |a| (a.start, a.end + 1 - a.start));
    }
    let (base, end) = crate::vm::kmem::handed_out();
    let mut va = if start { base } else { SWEEP_VA.load(Ordering::Relaxed) };
    if start {
        SWEEP_LAST.store(u64::MAX, Ordering::Relaxed);
        SWEEP_TABLE.store(0, Ordering::Relaxed);
    }
    let frame = |va: u64| crate::arch::mmu::kernel_frame(va).filter(|&pa| dumpable(pa));
    while va < end {
        if let Some(pa) = frame(va)
            && SWEEP_LAST.swap(pa, Ordering::Relaxed) != pa
        {
            let mut len = PAGE;
            while va + len < end && frame(va + len) == Some(pa + len) {
                len += PAGE;
            }
            SWEEP_VA.store(va + len, Ordering::Relaxed);
            return (pa, len);
        }
        va += PAGE;
    }
    SWEEP_VA.store(end, Ordering::Relaxed);
    let mut cursor = SWEEP_TABLE.load(Ordering::Relaxed);
    let table = crate::arch::mmu::next_table_frame(&mut cursor);
    SWEEP_TABLE.store(cursor, Ordering::Relaxed);
    table.map_or((u64::MAX, 0), |pa| (pa, PAGE))
}

/// Whether a frame the kernel maps belongs in a filtered dump: RAM (not a
/// device's registers, which a read may disturb), outside the image
/// filesystem (the dumper takes the bootstrap executables from it itself).
fn dumpable(pa: u64) -> bool {
    let sp = crate::syspage::get();
    let within = |name: &str| sp.asinfo().any(|a| sp.string(a.name) == name && (a.start..=a.end).contains(&pa));
    within("sysram") && !within("imagefs")
}

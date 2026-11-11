//! Starting the secondary CPUs.
//!
//! The kernel starts on the boot CPU only. It expects every other CPU to
//! wait in the `smp_spin` callout, with the MMU on, until it posts the
//! address of its per-CPU start code in the system page (`smp.start_addr`).
//! Getting them there takes two steps, as in QNX's startup library:
//!
//! 1. *Start*: while startup builds the system page, the boot CPU releases
//!    each secondary CPU in turn (how is the board's business: on the
//!    Raspberry Pi, a spin table) and waits for it. The CPU moves to EL1,
//!    describes itself in cpuinfo, sets up its view of the interrupt
//!    controller, and waits.
//! 2. *Transfer*: once the system page is in place, the boot CPU lets the
//!    CPUs go one at a time. Each turns its MMU on, enters `smp_spin`, and
//!    clears `smp.pending`, which tells the boot CPU to let the next go.
//!
//! Until the boot CPU turns its own MMU on, it reads memory uncached;
//! `smp_spin` cleans the line it writes so that the boot CPU sees it.

use super::handoff::{MmuRegs, vstart};
use super::{cache, cpu, exceptions, intr};
use crate::abi::syspage::SyspageEntry;
use crate::global::Global;
use crate::syspage::{MAX_CPUS, Syspage};
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Size of each secondary CPU's stack.
const STACK_SIZE: usize = 8 * 1024;

#[repr(C, align(16))]
struct Stacks([[u8; STACK_SIZE]; MAX_CPUS]);

#[unsafe(no_mangle)]
static mut SECONDARY_STACKS: Stacks = Stacks([[0; STACK_SIZE]; MAX_CPUS]);

/// The CPU being started, plus one; 0 once it has described itself.
#[unsafe(no_mangle)]
static CPU_STARTING: AtomicUsize = AtomicUsize::new(0);
/// The last CPU allowed to go to the kernel.
static CPU_RELEASED: AtomicUsize = AtomicUsize::new(0);

/// What the secondary CPUs need to enter the kernel, set by the boot CPU
/// before it releases them.
pub struct Transfer {
    pub syspage: u64,
    pub smp_spin: u64,
    pub tcr: u64,
    pub ttbr0: u64,
    /// Each CPU's TTBR1.
    pub ttbr1: [u64; MAX_CPUS],
    /// Physical address of `smp.pending` in the system page.
    pub pending: u64,
}

static TRANSFER: Global<Option<Transfer>> = Global::new(None);
/// The system page being built, for the secondary CPUs to describe
/// themselves in.
static SYSPAGE: AtomicU64 = AtomicU64::new(0);
static SPEED_MHZ: AtomicU64 = AtomicU64::new(0);

// ANCHOR: smp_spin
// The secondary CPUs' entry into the kernel: smp_spin(syspage, cpu).
// Offsets in the system page: the smp section's entry_off (36), and in it
// pending (0) and start_addr (16); see the layout test below.
crate::callout!(pub static SMP_SPIN, "oscraft_smp_spin", patches = 0,
    "ldrh    w2, [x0, #36]",
    "add     x2, x0, x2",
    // Tell the boot CPU, which reads memory uncached, that this CPU is
    // here.
    "str     wzr, [x2]",
    "dc      cvac, x2",
    "dsb     sy",
    // Wait for start_addr and take it, so that one CPU takes each value.
    "add     x3, x2, #16",
    "0: ldaxr x4, [x3]",
    "cbnz    x4, 1f",
    "wfe",
    "b       0b",
    "1: stlxr w5, xzr, [x3]",
    "cbnz    w5, 0b",
    // -1: the kernel shuts the CPUs down. Put it back for the others.
    "cmn     x4, #1",
    "b.eq    2f",
    "br      x4",
    "2: str  x4, [x3]",
    "dsb     sy",
    "sev",
    "3: wfi",
    "b       3b",
);
// ANCHOR_END: smp_spin

// ANCHOR: secondary_entry
// Where a released CPU starts, at its physical address, MMU off. Its
// logical number is CPU_STARTING - 1.
global_asm!(
    ".section .text.__oscraft_secondary_start, \"ax\"",
    ".global __oscraft_secondary_start",
    "__oscraft_secondary_start:",
    "    msr     daifset, #0xf",
    "    bl      __oscraft_to_el1",
    "    adrp    x9, CPU_STARTING",
    "    ldr     x19, [x9, :lo12:CPU_STARTING]",
    "    sub     x19, x19, #1",
    // Stack: the top of SECONDARY_STACKS[cpu].
    "    adrp    x9, SECONDARY_STACKS",
    "    add     x9, x9, :lo12:SECONDARY_STACKS",
    "    add     x10, x19, #1",
    "    mov     x11, #{stack_size}",
    "    madd    x9, x10, x11, x9",
    "    mov     sp, x9",
    "    mov     x0, x19",
    "    bl      __oscraft_secondary_main",
    "0:  wfe",
    "    b       0b",
    stack_size = const STACK_SIZE,
);
// ANCHOR_END: secondary_entry

unsafe extern "C" {
    fn __oscraft_secondary_start();
}

/// Physical address of the code a secondary CPU starts at.
pub fn secondary_entry() -> u64 {
    __oscraft_secondary_start as *const () as u64
}

// ANCHOR: secondary_main
#[unsafe(no_mangle)]
extern "C" fn __oscraft_secondary_main(cpu: usize) -> ! {
    exceptions::install();
    cache::clean_invalidate_dcache();
    cache::invalidate_icache();
    // SAFETY: the boot CPU waits while this CPU describes itself, so the
    // system page is this CPU's to write until it clears CPU_STARTING.
    let sp = unsafe { &mut *(SYSPAGE.load(Ordering::Acquire) as *mut Syspage) };
    cpu::init_cpuinfo(sp, cpu, SPEED_MHZ.load(Ordering::Relaxed) as u32);
    intr::init_cpu(&mut sp.gic_map, cpu, sp.num_cpu);
    cpu::init_timer();
    CPU_STARTING.store(0, Ordering::Release);

    while CPU_RELEASED.load(Ordering::Acquire) < cpu {
        core::hint::spin_loop();
    }
    // SAFETY: the boot CPU set TRANSFER before releasing any CPU.
    let Some(t) = (unsafe { (*TRANSFER.get()).as_ref() }) else {
        crate::crash!("cpu{cpu}: released with nowhere to go");
    };
    let regs = MmuRegs::new(t.tcr, t.ttbr0, t.ttbr1[cpu]);
    // SAFETY: the same page tables as the boot CPU's, with this CPU's L0.
    unsafe { vstart(t.syspage, t.smp_spin, cpu, &regs) }
}
// ANCHOR_END: secondary_main

// ANCHOR: start_cpus
/// Start CPUs 1 to `num_cpu - 1` (step 1): `release(cpu, entry)` lets a
/// CPU run from the physical address `entry`. Returns false, after a
/// message, if a CPU does not come.
pub fn start_cpus(sp: &mut Syspage, speed_mhz: u32, release: fn(usize, u64)) -> bool {
    SYSPAGE.store(sp as *mut Syspage as u64, Ordering::Release);
    SPEED_MHZ.store(u64::from(speed_mhz), Ordering::Relaxed);
    for cpu in 1..usize::from(sp.num_cpu) {
        CPU_STARTING.store(cpu + 1, Ordering::Release);
        release(cpu, secondary_entry());
        let mut spins = 0u64;
        while CPU_STARTING.load(Ordering::Acquire) != 0 {
            spins += 1;
            if spins == 1 << 28 {
                crate::kprintln!("startup: cpu{cpu} did not start");
                return false;
            }
            core::hint::spin_loop();
        }
    }
    true
}

/// Let the started CPUs enter the kernel (step 2), one at a time: each
/// clears `smp.pending` from `smp_spin`.
pub fn transfer_cpus(transfer: Transfer, num_cpu: usize) {
    let pending = transfer.pending as *mut u32;
    // SAFETY: no secondary CPU reads TRANSFER before CPU_RELEASED says so.
    unsafe { *TRANSFER.get() = Some(transfer) };
    for cpu in 1..num_cpu {
        // SAFETY: smp.pending, in the system page's memory.
        unsafe { pending.write_volatile(1) };
        CPU_RELEASED.store(cpu, Ordering::Release);
        // SAFETY: as above; the CPU clears it, uncached.
        while unsafe { pending.read_volatile() } != 0 {
            core::hint::spin_loop();
        }
    }
}
// ANCHOR_END: start_cpus

/// `smp_spin` hardcodes these offsets.
const _: () = {
    assert!(core::mem::offset_of!(SyspageEntry, smp) == 36);
    assert!(core::mem::offset_of!(crate::abi::syspage::SmpEntry, pending) == 0);
    assert!(core::mem::offset_of!(crate::abi::syspage::SmpEntry, start_addr) == 16);
};

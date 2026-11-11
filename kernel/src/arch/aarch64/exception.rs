//! Exception handling: vector installation, the saved context and the
//! dispatcher called from `vectors.s`.

use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicBool, Ordering};

use crate::kprintln;

global_asm!(include_str!("vectors.s"));

// ANCHOR: trap_frame
/// Context saved by `vectors.s`; layout shared with the assembly.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub sp_el0: u64,
    pub elr: u64,
    pub spsr: u64,
    pub esr: u64,
    pub far: u64,
}

const _: () = assert!(core::mem::size_of::<TrapFrame>() == 288);
// ANCHOR_END: trap_frame

impl TrapFrame {
    /// Initial state of a user thread: `entry(arg)` at EL0 (SPSR = EL0t,
    /// interrupts enabled) with stack pointer `sp`.
    pub fn new_user(entry: u64, sp: u64, arg: u64) -> Self {
        let mut x = [0; 31];
        x[0] = arg;
        Self {
            x,
            sp_el0: sp,
            elr: entry,
            spsr: 0,
            esr: 0,
            far: 0,
        }
    }

    /// Kernel call number (w8).
    pub fn kercall_number(&self) -> u32 {
        self.x[8] as u32
    }

    /// Kernel call argument `i` (x0..x7).
    pub fn kercall_arg(&self, i: usize) -> u64 {
        self.x[i]
    }

    // ANCHOR: complete_kercall
    /// Return from a kernel call: on success x0 = value at the normal
    /// return point; on failure x0 = errno, one instruction later.
    pub fn complete_kercall(&mut self, result: Result<u64, u32>) {
        match result {
            Ok(v) => self.x[0] = v,
            Err(e) => {
                self.x[0] = e as u64;
                self.elr += 4;
            }
        }
    }
    // ANCHOR_END: complete_kercall
}

/// `svc` immediate of QNX kernel calls.
const SVC_KERCALL: u64 = 0x51;
/// ESR_EL1.EC for an SVC from AArch64.
const EC_SVC64: u64 = 0x15;
const EC_FP_ACCESS: u64 = 0x07;
/// Instruction and data aborts from EL0.
const EC_IABT_LOW: u64 = 0x20;
const EC_DABT_LOW: u64 = 0x24;

const KIND_NAMES: [&str; 4] = ["synchronous", "IRQ", "FIQ", "SError"];
const SOURCE_NAMES: [&str; 4] = ["EL1 (SP_EL0)", "EL1", "EL0 (AArch64)", "EL0 (AArch32)"];

/// `ESR_EL1.EC` values we name in reports.
fn exception_class(ec: u64) -> &'static str {
    match ec {
        0x00 => "unknown reason",
        0x01 => "WFI/WFE",
        0x07 => "FP/SIMD access",
        0x0e => "illegal execution state",
        0x15 => "SVC (AArch64)",
        0x18 => "MSR/MRS/system instruction",
        0x20 => "instruction abort from EL0",
        0x21 => "instruction abort",
        0x22 => "PC alignment fault",
        0x24 => "data abort from EL0",
        0x25 => "data abort",
        0x26 => "SP alignment fault",
        0x2f => "SError",
        0x3c => "BRK",
        _ => "other",
    }
}

/// `brk` immediate used by the boot-time self-test.
const BRK_SELFTEST: u64 = 0x5e1f;
static SELFTEST_HIT: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    static exception_vectors: u8;
}

/// Install the vector table on this CPU.
pub fn init() {
    // SAFETY: the table is 2 KiB aligned code that follows the vector ABI.
    unsafe {
        let base = core::ptr::addr_of!(exception_vectors) as u64;
        asm!("msr vbar_el1, {}", "isb", in(reg) base, options(nostack));
    }
}

pub fn vector_base() -> u64 {
    let v: u64;
    // SAFETY: reading VBAR_EL1 has no side effects.
    unsafe { asm!("mrs {}, vbar_el1", out(reg) v, options(nomem, nostack)) };
    v
}

/// Take an exception on purpose and check that the vectors return from it.
pub fn selftest() -> bool {
    SELFTEST_HIT.store(false, Ordering::SeqCst);
    // SAFETY: the handler recognises this immediate and skips the instruction.
    unsafe { asm!("brk #{imm}", imm = const BRK_SELFTEST) };
    SELFTEST_HIT.load(Ordering::SeqCst)
}

#[unsafe(no_mangle)]
extern "C" fn exception_entry(frame: &mut TrapFrame, kind: u64) {
    // From EL0 with alignment checks on: off, before kernel code (which
    // may access memory misaligned) goes on; back on, if the thread
    // asked for them, as the last thing before the return to EL0.
    if kind < 8 {
        return handle(frame, kind);
    }
    if crate::percpu::this().align_check.load(Ordering::Relaxed) {
        set_align_check(false);
    }
    handle(frame, kind);
    set_align_check(crate::sched::current().align_fault.load(Ordering::Relaxed));
}

// A copy that may abort: memory outside RAM (a device behind a memory
// object, `SHMCTL_PHYS`) can refuse an access with an external abort. Its
// loads and stores are only in `fault_copy`; an abort there resumes at
// `fault_copy_fixup`, which returns 1, instead of being a kernel fault.
// Aligned 8- or 4-byte accesses where all three operands allow them, as
// device registers want, else bytes.
core::arch::global_asm!(
    ".text",
    ".balign 4",
    ".global oscraft_fault_copy",
    "oscraft_fault_copy:",
    "    orr     x3, x0, x1",
    "    orr     x3, x3, x2",
    "    tst     x3, #7",
    "    b.ne    3f",
    "2:  cbz     x2, 9f",
    "    ldr     x4, [x1], #8",
    "    str     x4, [x0], #8",
    "    sub     x2, x2, #8",
    "    b       2b",
    "3:  tst     x3, #3",
    "    b.ne    5f",
    "4:  cbz     x2, 9f",
    "    ldr     w4, [x1], #4",
    "    str     w4, [x0], #4",
    "    sub     x2, x2, #4",
    "    b       4b",
    "5:  cbz     x2, 9f",
    "    ldrb    w4, [x1], #1",
    "    strb    w4, [x0], #1",
    "    sub     x2, x2, #1",
    "    b       5b",
    "9:  mov     x0, #0",
    "    ret",
    ".global oscraft_fault_copy_fixup",
    "oscraft_fault_copy_fixup:",
    "    mov     x0, #1",
    "    ret",
    ".global oscraft_fault_copy_end",
    "oscraft_fault_copy_end:",
);

unsafe extern "C" {
    fn oscraft_fault_copy(dst: *mut u8, src: *const u8, len: usize) -> u64;
    fn oscraft_fault_copy_fixup();
    fn oscraft_fault_copy_end();
}

/// Copy `len` bytes from `src` to `dst`; false if an access was refused
/// with an abort (a device's register), which then ends the copy.
///
/// # Safety
///
/// Both ranges must be mapped for the kernel, `dst` writable; neither may
/// overlap the other.
pub unsafe fn fault_copy(dst: *mut u8, src: *const u8, len: usize) -> bool {
    // SAFETY: as the caller promises; the routine touches only the ranges.
    unsafe { oscraft_fault_copy(dst, src, len) == 0 }
}

/// A data abort taken inside `fault_copy`: resume at its fixup.
fn fault_copy_abort(frame: &mut TrapFrame) -> bool {
    let start = oscraft_fault_copy as *const () as u64;
    let end = oscraft_fault_copy_end as *const () as u64;
    if !(start..end).contains(&frame.elr) {
        return false;
    }
    frame.elr = oscraft_fault_copy_fixup as *const () as u64;
    true
}

fn handle(frame: &mut TrapFrame, kind: u64) {
    let ec = frame.esr >> 26;
    let iss = frame.esr & 0x1ff_ffff;
    if kind & 3 == 0 && ec == 0x3c && iss == BRK_SELFTEST {
        SELFTEST_HIT.store(true, Ordering::SeqCst);
        frame.elr += 4;
        return;
    }
    // A device refused an access of `fault_copy` (kernel, data abort).
    if kind == 4 && ec == 0x25 && fault_copy_abort(frame) {
        return;
    }
    // Kernel call from EL1 (kernel threads) or EL0.
    if (kind == 4 || kind == 8) && ec == EC_SVC64 && frame.esr & 0xffff == SVC_KERCALL {
        crate::ker::dispatch(frame);
        crate::sched::preempt();
        return;
    }
    // FP/SIMD trapped at EL0: the thread's first FP instruction since it
    // was switched in loads its state (lazy FP), and runs again.
    if kind == 8 && ec == EC_FP_ACCESS {
        crate::sched::fp_make_live();
        return;
    }
    // A translation fault in a lazily allocated region (the main stack):
    // its page is allocated, and the access done again. A write to a
    // read-only page of a writable region: a copy-on-write page since a
    // fork, made the process's own. In the syndrome, bit 6 (WnR) is set for
    // a write, and bits 5:2 of the fault status code are 0b0001 for a
    // translation fault and 0b0011 for a permission fault (bits 1:0 are the
    // table level, hence the mask 0x3c).
    let write = ec == EC_DABT_LOW && frame.esr & (1 << 6) != 0;
    if kind == 8
        && matches!(ec, EC_IABT_LOW | EC_DABT_LOW)
        && (frame.esr & 0x3c == 0x04 || (write && frame.esr & 0x3c == 0x0c))
        && crate::vm::map::fault_in(
            &crate::sched::current().process,
            frame.far,
            write,
        )
    {
        return;
    }
    // Any other synchronous exception from EL0 is a fault of that thread.
    // Unless its debugger wants to see it: the process stops there.
    if kind == 8 {
        if crate::ker::debug::fault(frame) {
            crate::ker::user_return(frame);
            if crate::ker::debug::fault_cleared(&crate::sched::current().process) {
                return;
            }
        }
        // Reported unless a handler takes it (and the program goes on).
        if !crate::ker::debug::fault_caught(frame) {
            report(frame, kind);
        }
        // A fault is its signal (a handler may take it); anything else
        // ends the thread.
        if crate::ker::debug::fault_to_signal(frame) {
            crate::ker::user_return(frame);
            return;
        }
        crate::ker::process::fault_current(exception_class(ec));
    }
    // IRQ taken from EL1 (SP_EL1) or from EL0.
    if kind == 5 || kind == 9 {
        let cpu = crate::percpu::this();
        cpu.in_irq.store(true, Ordering::Relaxed);
        crate::interrupt::handle();
        cpu.in_irq.store(false, Ordering::Relaxed);
        crate::sched::preempt();
        if kind == 9 {
            // Back to EL0: a thread busy in user code also ends or takes
            // its signals.
            crate::ker::user_return(frame);
        }
        return;
    }
    report(frame, kind);
    // A kernel debugger or dumper first (QNX's kdumper), with the faulting
    // registers.
    let (signo, fault) = match ec {
        0x20 | 0x21 | 0x24 | 0x25 => (crate::kdebug::SIGSEGV, crate::kdebug::FLTPAGE),
        _ => (crate::kdebug::SIGILL, crate::kdebug::FLTILL),
    };
    crate::kdebug::crash(Some(frame), crate::kdebug::sigcode(signo, 1, fault));
    panic!("unhandled exception");
}

/// The caller's registers, as a crash reports them (x0..x30, sp; the
/// return address as the pc; the current PSTATE's DAIF and EL bits).
pub fn capture_registers(gpr: &mut [u64; 32], pc: &mut u64, pstate: &mut u64) {
    let (sp, lr, fp, daif, el): (u64, u64, u64, u64, u64);
    // SAFETY: register reads only.
    unsafe {
        core::arch::asm!(
            "mov {sp}, sp", "mov {lr}, x30", "mov {fp}, x29",
            "mrs {daif}, daif", "mrs {el}, CurrentEL",
            sp = out(reg) sp, lr = out(reg) lr, fp = out(reg) fp,
            daif = out(reg) daif, el = out(reg) el,
            options(nomem, nostack)
        )
    };
    gpr[29] = fp;
    gpr[30] = lr;
    gpr[31] = sp;
    *pc = lr;
    // EL1h (SPSel 1) with the current DAIF.
    *pstate = daif | el | 1;
}

/// Alignment checks (`SCTLR_EL1.A`) on or off. They apply at EL1 too, so
/// they are on only from the return to EL0 of a thread that asked for
/// them (`_NTO_TCTL_ALIGN_FAULT`) to its next exception.
pub fn set_align_check(on: bool) {
    let cpu = crate::percpu::this();
    if cpu.align_check.swap(on, Ordering::Relaxed) == on {
        return;
    }
    // SAFETY: only changes whether misaligned accesses fault.
    unsafe {
        core::arch::asm!(
            "mrs {t}, sctlr_el1",
            "bic {t}, {t}, #2",
            "orr {t}, {t}, {a}",
            "msr sctlr_el1, {t}",
            "isb",
            t = out(reg) _,
            a = in(reg) (on as u64) << 1,
            options(nostack),
        );
    }
}

/// Fault status code (`ESR_EL1.ISS[5:0]`) of instruction and data aborts.
fn fault_status(fsc: u64) -> &'static str {
    match fsc {
        0x00..=0x03 => "address size fault",
        0x04..=0x07 => "translation fault",
        0x09..=0x0b => "access flag fault",
        0x0d..=0x0f => "permission fault",
        0x10 => "synchronous external abort",
        0x21 => "alignment fault",
        _ => "other fault",
    }
}

fn report(frame: &TrapFrame, kind: u64) {
    let ec = frame.esr >> 26;
    if matches!(ec, 0x20 | 0x21 | 0x24 | 0x25) {
        let fsc = frame.esr & 0x3f;
        kprintln!("{} at level {} (FSC {fsc:#x})", fault_status(fsc), fsc & 3);
    }
    kprintln!(
        "{} exception from {}: {} (EC {:#x}), ESR {:#x}",
        KIND_NAMES[(kind & 3) as usize],
        SOURCE_NAMES[(kind >> 2) as usize],
        exception_class(ec),
        ec,
        frame.esr
    );
    kprintln!(
        "  ELR {:#018x}  FAR {:#018x}  SPSR {:#x}  SP_EL0 {:#x}",
        frame.elr,
        frame.far,
        frame.spsr,
        frame.sp_el0
    );
    for (i, pair) in frame.x.chunks(2).enumerate() {
        match pair {
            [a, b] => kprintln!("  x{:<2} {a:#018x}  x{:<2} {b:#018x}", 2 * i, 2 * i + 1),
            [a] => kprintln!("  x{:<2} {a:#018x}", 2 * i),
            _ => {}
        }
    }
    // Which thread, on which kernel stack: a fault in the page below that
    // stack (its guard) is the stack overflowing.
    let t = crate::sched::current();
    let (base, pages) = t.kernel_stack();
    let page = crate::vm::pmem::PAGE_SIZE;
    kprintln!(
        "  thread {} ({}, pid {}), kernel stack {:#x}..{:#x}",
        t.tid,
        t.name,
        t.process.pid,
        base,
        base + pages * page
    );
    let guard = crate::arch::mmu::boot_stack_guard();
    if pages != 0 && (base - page..base).contains(&frame.far) || (guard..guard + page).contains(&frame.far) {
        kprintln!("  the fault is in the stack's guard page: kernel stack overflow");
    }
}

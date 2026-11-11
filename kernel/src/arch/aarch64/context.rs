//! Saved kernel thread context and the context switch.

use core::arch::global_asm;

/// Set the calling thread's user thread pointer (TPIDR_EL0). While it is
/// in the kernel the register holds its value; switches save it.
pub fn set_user_tp(tp: u64) {
    // SAFETY: TPIDR_EL0 is only the user's thread pointer to the kernel.
    unsafe { core::arch::asm!("msr tpidr_el0, {}", in(reg) tp, options(nomem, nostack)) };
}

global_asm!(include_str!("context.s"));

unsafe extern "C" {
    fn cpu_switch(old: *mut Context, new: *const Context, old_on_cpu: *mut u8, new_on_cpu: *mut u8);
    fn thread_start();
    fn thread_start_user();
    fn fpsimd_save(state: *mut FpState);
    fn fpsimd_restore(state: *const FpState);
}

/// A user thread's FP/SIMD registers: v0..v31, FPCR, FPSR.
#[repr(C, align(16))]
#[derive(Clone)]
pub struct FpState {
    regs: [u64; 66],
}

/// Size of QNX's `AARCH64_FPU_REGISTERS` (`<aarch64/context.h>`): q0-q31,
/// then fpsr and fpcr as 32-bit words.
pub const QNX_FPU_SIZE: usize = 520;

impl FpState {
    pub const fn new() -> Self {
        Self { regs: [0; 66] }
    }

    /// In the layout of QNX's `AARCH64_FPU_REGISTERS`.
    pub fn to_qnx(&self) -> [u8; QNX_FPU_SIZE] {
        let mut b = [0u8; QNX_FPU_SIZE];
        for (i, r) in self.regs[..64].iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&r.to_le_bytes());
        }
        // Ours: fpcr at 512, fpsr at 520; QNX's: fpsr at 512, fpcr at 516.
        b[512..516].copy_from_slice(&(self.regs[65] as u32).to_le_bytes());
        b[516..520].copy_from_slice(&(self.regs[64] as u32).to_le_bytes());
        b
    }

    /// From the layout of QNX's `AARCH64_FPU_REGISTERS`.
    pub fn from_qnx(b: &[u8; QNX_FPU_SIZE]) -> Self {
        let mut s = Self::new();
        for i in 0..64 {
            s.regs[i] = u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        }
        s.regs[65] = u32::from_le_bytes(b[512..516].try_into().unwrap()) as u64;
        s.regs[64] = u32::from_le_bytes(b[516..520].try_into().unwrap()) as u64;
        s
    }
}

/// Save this CPU's FP/SIMD registers into `state`.
pub fn fp_save(state: &mut FpState) {
    // SAFETY: stores the registers into `state`; the kernel does not use them.
    unsafe { fpsimd_save(state) }
}

/// Load `state` into this CPU's FP/SIMD registers.
pub fn fp_restore(state: &FpState) {
    // SAFETY: loads registers only the running user thread will see.
    unsafe { fpsimd_restore(state) }
}

/// Allow FP/SIMD at EL0 and EL1 on this CPU (CPACR_EL1.FPEN = 0b11). The
/// kernel itself is built without FP; this is for user threads.
pub fn fp_enable_cpu() {
    // SAFETY: only changes FP/SIMD trapping.
    unsafe {
        core::arch::asm!(
            "mrs {t}, cpacr_el1",
            "orr {t}, {t}, #(3 << 20)",
            "msr cpacr_el1, {t}",
            "isb",
            t = out(reg) _,
            options(nostack)
        );
    }
}

/// Trap (`on`) or allow EL0's FP/SIMD instructions on this CPU (CPACR_EL1.FPEN
/// 0b01 or 0b11); EL1 may use the registers either way (to save and restore
/// them).
pub fn fp_set_el0_trap(on: bool) {
    let fpen: u64 = if on { 1 << 20 } else { 3 << 20 };
    // SAFETY: only changes FP/SIMD trapping.
    unsafe {
        // Written only when it changes, as for MDSCR_EL1 (`set_step`).
        core::arch::asm!(
            "mrs {t}, cpacr_el1",
            "bic {u}, {t}, #(3 << 20)",
            "orr {u}, {u}, {f}",
            "cmp {u}, {t}",
            "b.eq 2f",
            "msr cpacr_el1, {u}",
            "isb",
            "2:",
            t = out(reg) _,
            u = out(reg) _,
            f = in(reg) fpen,
            options(nostack)
        );
    }
}

/// x19..x28, x29 (fp), x30 (lr), sp, TPIDR_EL0; layout shared with context.s.
#[repr(C)]
#[derive(Debug, Default)]
pub struct Context {
    regs: [u64; 14],
}

impl Context {
    /// Context of a new thread: it starts in `thread_start`, which calls
    /// `thread_entry(entry, arg)` on the given stack.
    pub fn new_thread(stack_top: u64, entry: u64, arg: u64) -> Self {
        let mut regs = [0; 14];
        regs[0] = entry; // x19
        regs[1] = arg; // x20
        regs[11] = thread_start as *const () as u64; // x30
        regs[12] = stack_top & !15; // sp
        Self { regs }
    }
}

impl Context {
    /// The saved user thread pointer (TPIDR_EL0) of a thread that is not
    /// running.
    pub fn user_tp(&self) -> u64 {
        self.regs[13]
    }

    /// Context of a new user thread whose initial user state is the
    /// TrapFrame at `frame` (top of its kernel stack), with user thread
    /// pointer `tp` (TPIDR_EL0).
    pub fn new_user_thread(frame: u64, tp: u64) -> Self {
        let mut regs = [0; 14];
        regs[0] = frame; // x19
        regs[11] = thread_start_user as *const () as u64; // x30
        regs[12] = frame; // sp
        regs[13] = tp; // TPIDR_EL0
        Self { regs }
    }
}

/// Switch from the running thread to another.
///
/// # Safety
///
/// `old` must be the running thread's context and `new` the saved context
/// of a thread no other CPU will switch to; IRQs must be masked.
/// `old_on_cpu` is cleared once `old` has been saved; then `new_on_cpu` is
/// waited for to be clear (the CPU that ran `new` last has saved it) and
/// set. Waiting only after giving up `old` keeps two CPUs, each switching
/// to the thread the other is leaving, from waiting for each other.
pub unsafe fn switch(old: *mut Context, new: *const Context, old_on_cpu: *mut u8, new_on_cpu: *mut u8) {
    // SAFETY: forwarded from the caller.
    unsafe { cpu_switch(old, new, old_on_cpu, new_on_cpu) }
}

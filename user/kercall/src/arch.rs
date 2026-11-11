//! AArch64.

use core::ffi::c_uint;
use core::sync::atomic::{AtomicU32, Ordering};

/// The body of a kernel call's C function on AArch64: the call's number in
/// `w8`, then `svc #0x51`. The kernel returns to the instruction after the
/// `svc` on success, and to the one after that on failure, with the error
/// number in `x0`. The three conventions differ only in that failure
/// path:
///
/// - `errno`: branch to `$err`, which sets `errno` and returns -1;
/// - `neg`: return the negated error number;
/// - `status`: return the error number itself, as a status (EOK, 0, is
///   success).
///
/// `neg(n, ...)` negates argument registers first: a negative part count
/// tells the kernel that a message is one buffer, not an I/O vector.
macro_rules! kercall_asm {
    (errno, $nr:path, $err:path $(, neg($($n:literal),+))?) => {
        core::arch::naked_asm!(
            $($(concat!("neg x", stringify!($n), ", x", stringify!($n)),)+)?
            "mov w8, #{nr}",
            "svc #0x51",
            "ret",
            "b {err}",
            nr = const $nr,
            err = sym $err,
        )
    };
    (neg, $nr:path, $err:path $(, neg($($n:literal),+))?) => {
        core::arch::naked_asm!(
            $($(concat!("neg x", stringify!($n), ", x", stringify!($n)),)+)?
            "mov w8, #{nr}",
            "svc #0x51",
            "ret",
            "neg x0, x0",
            "ret",
            nr = const $nr,
        )
    };
    (status, $nr:path, $err:path $(, neg($($n:literal),+))?) => {
        core::arch::naked_asm!(
            $($(concat!("neg x", stringify!($n), ", x", stringify!($n)),)+)?
            "mov w8, #{nr}",
            "svc #0x51",
            "ret",
            "ret",
            nr = const $nr,
        )
    };
}
pub(crate) use kercall_asm;

/// The thread pointer, `TPIDR_EL0`: the kernel points it at the thread's
/// control block.
#[inline]
pub fn thread_pointer() -> *mut u8 {
    let tp: *mut u8;
    // SAFETY: reads a register; no memory is touched.
    unsafe {
        core::arch::asm!("mrs {}, tpidr_el0", out(reg) tp, options(nomem, nostack, preserves_flags))
    };
    tp
}

/// The free-running counter, `CNTVCT_EL0`: `ClockCycles()`. The `isb`
/// keeps the read in program order.
#[inline]
pub fn clock_cycles() -> u64 {
    let count: u64;
    // SAFETY: reads a register; no memory is touched.
    unsafe {
        core::arch::asm!("isb", "mrs {}, cntvct_el0", out(reg) count, options(nomem, nostack, preserves_flags))
    };
    count
}

/// Masks the CPU's interrupts (IRQs). Needs I/O privileges
/// (`ThreadCtl(_NTO_TCTL_IO, ...)`).
#[inline]
pub fn interrupt_disable() {
    // SAFETY: changes the interrupt mask only; it is a compiler barrier.
    unsafe { core::arch::asm!("msr daifset, #2", options(nostack, preserves_flags)) };
}

/// Unmasks the CPU's interrupts (IRQs).
#[inline]
pub fn interrupt_enable() {
    // SAFETY: as above.
    unsafe { core::arch::asm!("msr daifclr, #2", options(nostack, preserves_flags)) };
}

/// Whether the CPU's interrupts are enabled: non-zero if they are.
#[inline]
pub fn interrupt_status() -> c_uint {
    let daif: u64;
    // SAFETY: reads a register; no memory is touched.
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack, preserves_flags))
    };
    (daif as c_uint & 0x80) ^ 0x80
}

/// `struct intrspin` (`<sys/neutrino.h>`): a spinlock word, 0 when free.
#[repr(C)]
pub struct IntrSpin {
    pub value: AtomicU32,
}

/// Masks interrupts, then takes `spin`: a lock shared between a thread
/// and an interrupt handler on another CPU (`<sys/neutrino.h>`).
///
/// # Safety
///
/// `spin` points to a valid `struct intrspin`; the caller has I/O
/// privileges.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn InterruptLock(spin: *mut IntrSpin) {
    interrupt_disable();
    // SAFETY: the caller's guarantee.
    let word = unsafe { &(*spin).value };
    while word
        .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

/// Releases `spin`, then unmasks interrupts.
///
/// # Safety
///
/// As [`InterruptLock`]; the caller holds `spin`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn InterruptUnlock(spin: *mut IntrSpin) {
    // SAFETY: the caller's guarantee.
    unsafe { (*spin).value.store(0, Ordering::Release) };
    interrupt_enable();
}

/// `InterruptDisable()`, as the exported function.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptDisable() {
    interrupt_disable();
}

/// `InterruptEnable()`, as the exported function.
#[unsafe(no_mangle)]
pub extern "C" fn InterruptEnable() {
    interrupt_enable();
}

/// Stops in the debugger: the breakpoint instruction gdb uses.
#[unsafe(no_mangle)]
pub extern "C" fn DebugBreak() {
    // SAFETY: a breakpoint; the debugger, or the kernel's SIGTRAP, decides.
    unsafe { core::arch::asm!("brk #0", options(nomem, nostack)) };
}

/// Stops in the kernel debugger.
#[unsafe(no_mangle)]
pub extern "C" fn DebugKDBreak() {
    // SAFETY: as above.
    unsafe { core::arch::asm!("brk #0xfffe", options(nomem, nostack)) };
}

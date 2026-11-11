//! The first instructions: `_start`, and the move to EL1.
//!
//! The boot loader enters the image at `_start` on the boot CPU, with the
//! MMU off and x0-x3 holding its arguments (on the Raspberry Pi, x0 is the
//! device tree's address). It may enter at EL3, EL2 or EL1. QNX runs its
//! kernel at EL1, so `_start` first moves the CPU there, then saves x0-x3,
//! sets up a stack and calls Rust.

use core::arch::global_asm;

/// Size of the boot CPU's stack.
const STACK_SIZE: usize = 16 * 1024;

#[repr(C, align(16))]
struct Stack([u8; STACK_SIZE]);

#[unsafe(no_mangle)]
static mut STARTUP_STACK: Stack = Stack([0; STACK_SIZE]);

/// x0-x3 as the boot loader left them.
#[unsafe(no_mangle)]
pub static mut BOOT_REGS: [u64; 4] = [0; 4];

// ANCHOR: start
global_asm!(
    ".section .text._start, \"ax\"",
    ".global _start",
    "_start:",
    // No interrupts until the kernel takes over.
    "    msr     daifset, #0xf",
    // Save the boot loader's arguments.
    "    adrp    x9, BOOT_REGS",
    "    add     x9, x9, :lo12:BOOT_REGS",
    "    stp     x0, x1, [x9]",
    "    stp     x2, x3, [x9, #16]",
    "    bl      __oscraft_to_el1",
    // A stack, at the top of STARTUP_STACK.
    "    adrp    x9, STARTUP_STACK",
    "    add     x9, x9, :lo12:STARTUP_STACK",
    "    add     x9, x9, #{stack_size}",
    "    mov     sp, x9",
    "    bl      __oscraft_cpu_init",
    // __oscraft_startup_main(&BOOT_REGS), defined by entry!().
    "    adrp    x0, BOOT_REGS",
    "    add     x0, x0, :lo12:BOOT_REGS",
    "    bl      __oscraft_startup_main",
    "0:  wfe",
    "    b       0b",
    stack_size = const STACK_SIZE,
);
// ANCHOR_END: start

// ANCHOR: to_el1
// __oscraft_to_el1: move the CPU from EL3 or EL2 to EL1 (or stay at EL1),
// with all exceptions masked, and return there. It uses no stack and only
// x9-x13, so that _start's x0-x3 and the secondary CPUs' arguments survive.
global_asm!(
    ".section .text.__oscraft_to_el1, \"ax\"",
    ".global __oscraft_to_el1",
    "__oscraft_to_el1:",
    "    mov     x13, x30",
    "    mrs     x9, CurrentEL",
    "    lsr     x9, x9, #2",
    "    cmp     x9, #3",
    "    b.ne    1f",
    // EL3: lower ELs are non-secure (NS), AArch64 (RW), HVC enabled (HCE),
    // and do not trap pointer authentication (API, APK) if the CPU has it.
    "    mov     x10, #{scr_el3}",
    "    bl      __oscraft_has_pauth",
    "    cbz     x11, 0f",
    "    orr     x10, x10, #{scr_pauth}",
    "0:  msr     scr_el3, x10",
    "    msr     cptr_el3, xzr",
    "    adr     x10, 1f",
    "    msr     elr_el3, x10",
    "    mov     x10, #{spsr_el2h}",
    "    msr     spsr_el3, x10",
    "    eret",
    "1:  mrs     x9, CurrentEL",
    "    lsr     x9, x9, #2",
    "    cmp     x9, #2",
    "    b.ne    2f",
    // EL2: a boot loader may have left the EL2 MMU and caches on (U-Boot
    // does); turn them off, as EL1 will run without them. _start cleans
    // the caches once it has a stack.
    "    mrs     x10, sctlr_el2",
    "    mov     x11, #{sctlr_mci}",
    "    bic     x10, x10, x11",
    "    msr     sctlr_el2, x10",
    "    isb",
    // EL1 is AArch64 (RW) and HVC is disabled (HCD); nothing traps to
    // EL2: not the FPU (CPTR_EL2), not the timers (CNTHCTL_EL2), not
    // pointer authentication (API, APK) if the CPU has it.
    "    ldr     x10, ={hcr_el2}",
    "    bl      __oscraft_has_pauth",
    "    cbz     x11, 0f",
    "    orr     x10, x10, #{hcr_pauth}",
    "0:  msr     hcr_el2, x10",
    "    msr     cptr_el2, xzr",
    "    msr     hstr_el2, xzr",
    "    mov     x10, #3",
    "    msr     cnthctl_el2, x10",
    "    msr     cntvoff_el2, xzr",
    "    msr     vttbr_el2, xzr",
    // EL1 reads its own IDs, not virtualized ones.
    "    mrs     x10, midr_el1",
    "    msr     vpidr_el2, x10",
    "    mrs     x10, mpidr_el1",
    "    msr     vmpidr_el2, x10",
    // EL1 starts with its MMU and caches off.
    "    ldr     x10, ={sctlr_el1}",
    "    msr     sctlr_el1, x10",
    "    adr     x10, 2f",
    "    msr     elr_el2, x10",
    "    mov     x10, #{spsr_el1h}",
    "    msr     spsr_el2, x10",
    "    eret",
    // EL1.
    "2:  msr     spsel, #1",
    "    msr     daifset, #0xf",
    "    isb",
    "    ret     x13",
    scr_el3 = const (1 << 10) | (1 << 8) | (1 << 0),
    scr_pauth = const (1 << 17) | (1 << 16),
    hcr_pauth = const (1u64 << 41) | (1 << 40),
    // SCTLR_EL2's M (MMU), C (data cache) and I (instruction cache).
    sctlr_mci = const (1 << 0) | (1 << 2) | (1 << 12),
    hcr_el2 = const (1u64 << 31) | (1 << 29),
    // SCTLR_EL1's RES1 bits (29, 28, 23, 22, 20, 11), MMU and caches off.
    sctlr_el1 = const 0x30d0_0800u64,
    // DAIF masked, return to EL2h / EL1h.
    spsr_el2h = const 0x3c9,
    spsr_el1h = const 0x3c5,
);
// ANCHOR_END: to_el1

// __oscraft_has_pauth: x11 = 1 if the CPU has pointer authentication
// (ID_AA64ISAR1_EL1.{APA, API, GPA, GPI}, ID_AA64ISAR2_EL1.{APA3, GPA3}),
// else 0. Uses x11 and x12 only, and no stack, for __oscraft_to_el1; it
// must not change x30's caller, so __oscraft_to_el1 saves x30 in x13.
global_asm!(
    ".section .text.__oscraft_has_pauth, \"ax\"",
    "__oscraft_has_pauth:",
    "    mov     x11, #1",
    "    mrs     x12, id_aa64isar1_el1",
    "    tst     x12, #(0xff << 4)",
    "    b.ne    1f",
    "    tst     x12, #(0xff << 24)",
    "    b.ne    1f",
    "    mrs     x12, id_aa64isar2_el1",
    "    tst     x12, #(0xff << 8)",
    "    b.ne    1f",
    "    mov     x11, #0",
    "1:  ret",
);

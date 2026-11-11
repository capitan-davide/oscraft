//! Exception vectors for startup itself.
//!
//! Startup takes no interrupts, so an exception means a bug: a bad
//! address, an undefined instruction. The vectors report it on the debug
//! console with the registers that say what happened, and stop.

use core::arch::global_asm;

// ANCHOR: vectors
// 16 entries of 0x80 bytes: {current EL with SP0, current EL with SPx,
// lower EL AArch64, lower EL AArch32} x {sync, IRQ, FIQ, SError}. Each
// entry passes its number to the common handler.
global_asm!(
    ".section .text.__oscraft_vectors, \"ax\"",
    ".balign 2048",
    ".global __oscraft_vectors",
    "__oscraft_vectors:",
    ".set n, 0",
    ".rept 16",
    "    .balign 128",
    "    mov     x0, #n",
    "    b       __oscraft_vector_common",
    "    .set n, n + 1",
    ".endr",
    "__oscraft_vector_common:",
    "    mrs     x1, esr_el1",
    "    mrs     x2, elr_el1",
    "    mrs     x3, far_el1",
    "    mrs     x4, spsr_el1",
    "    mov     x5, sp",
    "    bl      __oscraft_exception",
);
// ANCHOR_END: vectors

const KINDS: [&str; 4] = ["synchronous", "IRQ", "FIQ", "SError"];

#[unsafe(no_mangle)]
extern "C" fn __oscraft_exception(
    vector: u64,
    esr: u64,
    elr: u64,
    far: u64,
    spsr: u64,
    sp: u64,
) -> ! {
    crate::crash!(
        "unexpected {} exception: ESR_EL1={esr:#x} (class {:#x}) ELR_EL1={elr:#x} FAR_EL1={far:#x} SPSR_EL1={spsr:#x} SP={sp:#x}",
        KINDS[(vector & 3) as usize],
        esr >> 26,
    )
}

/// Install the vectors on this CPU.
pub fn install() {
    // SAFETY: the vectors are 2 KB aligned code; startup runs at EL1.
    unsafe {
        core::arch::asm!(
            "adr {t}, __oscraft_vectors",
            "msr vbar_el1, {t}",
            "isb",
            t = out(reg) _,
        )
    };
}

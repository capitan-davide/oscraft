/*
 * EL1 exception vectors.
 *
 * Every entry saves the interrupted context as a `TrapFrame` (see
 * exception.rs) on the current stack, calls exception_entry(frame, kind)
 * and restores the (possibly modified) context. `kind` is the vector index:
 * 4 * source + type, source = current EL with SP_EL0 / with SP_ELx / lower
 * EL AArch64 / lower EL AArch32, type = sync / IRQ / FIQ / SError.
 */

    .equ    FRAME_SIZE, 288         /* x0-x30, sp_el0, elr, spsr, esr, far */

    .macro  VECTOR kind
    .balign 128
    sub     sp, sp, #FRAME_SIZE
    stp     x0, x1, [sp, #0]        /* saved first: x0 is needed for kind */
    mov     x0, #\kind
    b       exception_common
    .endm

    /* A synchronous exception at EL1 on SP_EL1 (kind 4) may be the kernel
     * stack overflowing into its guard page: pushing the frame there would
     * fault again, for ever. So the frame's page is first checked with
     * AT S1E1W, x0 parked meanwhile in TPIDRRO_EL0 (which the kernel
     * doesn't use, and which is zero again before anything runs). If it
     * isn't writable, kernel_stack_overflow takes over (review CORE-17). */
    .macro  VECTOR_EL1_SYNC
    .balign 128
    msr     tpidrro_el0, x0
    mov     x0, sp
    sub     x0, x0, #FRAME_SIZE
    at      s1e1w, x0
    isb
    mrs     x0, par_el1
    tbnz    x0, #0, kernel_stack_overflow
    mrs     x0, tpidrro_el0
    msr     tpidrro_el0, xzr
    sub     sp, sp, #FRAME_SIZE
    stp     x0, x1, [sp, #0]
    mov     x0, #4
    b       exception_common
    .endm

    .section .text.vectors, "ax"
    .balign 2048                    /* as VBAR_EL1 requires */
    .global exception_vectors
exception_vectors:
    VECTOR  0
    VECTOR  1
    VECTOR  2
    VECTOR  3
    VECTOR_EL1_SYNC
    VECTOR  5
    VECTOR  6
    VECTOR  7
    VECTOR  8
    VECTOR  9
    VECTOR  10
    VECTOR  11
    VECTOR  12
    VECTOR  13
    VECTOR  14
    VECTOR  15

    .text
    /* The kernel stack ran into its guard page (from VECTOR_EL1_SYNC, x0's
     * value in TPIDRRO_EL0): the handler runs on the emergency stack, and
     * reports the overflow (exception.rs). The faulting sp goes to SP_EL0,
     * which the frame records (sp_el0). Fatal, so one emergency stack for
     * all CPUs: two overflowing at once would garble the report only. */
kernel_stack_overflow:
    mov     x0, sp
    msr     sp_el0, x0
    adrp    x0, emergency_stack_top
    add     x0, x0, :lo12:emergency_stack_top
    mov     sp, x0
    mrs     x0, tpidrro_el0
    msr     tpidrro_el0, xzr
    sub     sp, sp, #FRAME_SIZE
    stp     x0, x1, [sp, #0]
    mov     x0, #4
    b       exception_common

    .section .bss.emergency_stack, "aw", %nobits
    .balign 16
emergency_stack:
    .space  16384
emergency_stack_top:

    .text
exception_common:
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    mrs     x2, sp_el0
    stp     x30, x2, [sp, #240]
    mrs     x2, elr_el1
    mrs     x3, spsr_el1
    stp     x2, x3, [sp, #256]
    mrs     x2, esr_el1
    mrs     x3, far_el1
    stp     x2, x3, [sp, #272]

    mov     x1, x0                  /* kind */
    mov     x0, sp                  /* frame */
    bl      exception_entry

    /* Restore the context in the TrapFrame at sp and return to it. New
     * user threads start here too (thread_start_user). */
    .global exception_return
exception_return:
    /* The system registers first, through x2 and x3, which get their own
     * values back below. */
    ldp     x2, x3, [sp, #256]
    msr     elr_el1, x2
    msr     spsr_el1, x3
    ldp     x30, x2, [sp, #240]
    msr     sp_el0, x2
    ldp     x0, x1, [sp, #0]
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    add     sp, sp, #FRAME_SIZE
    eret

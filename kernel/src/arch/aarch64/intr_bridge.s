/*
 * Run an interrupt callout fragment (a copy of a system page id or eoi
 * code fragment, followed by `ret`).
 *
 * u64 intr_fragment(code, state)
 *   x0 = code to call
 *   x1 = struct FragmentState: x19..x28 as the fragment sees them
 *
 * Fragments communicate through callee-saved registers (x19 = interrupt
 * id, x20 = system page, x22 = mask count, x24 = state shared between the
 * fragments of one interrupt). They are loaded from `state` before the call
 * and stored back after it, so a sequence of calls behaves like one inline
 * code path. Returns x19.
 */

    .text
    .global intr_fragment
intr_fragment:
    stp     x29, x30, [sp, #-96]!
    mov     x29, sp
    stp     x19, x20, [sp, #16]
    stp     x21, x22, [sp, #32]
    stp     x23, x24, [sp, #48]
    stp     x25, x26, [sp, #64]
    stp     x27, x28, [sp, #80]

    mov     x9, x0
    mov     x10, x1
    ldp     x19, x20, [x10, #0]
    ldp     x21, x22, [x10, #16]
    ldp     x23, x24, [x10, #32]
    ldp     x25, x26, [x10, #48]
    ldp     x27, x28, [x10, #64]
    str     x10, [sp, #-16]!        /* keep the state pointer */
    blr     x9
    ldr     x10, [sp], #16
    stp     x19, x20, [x10, #0]
    stp     x21, x22, [x10, #16]
    stp     x23, x24, [x10, #32]
    stp     x25, x26, [x10, #48]
    stp     x27, x28, [x10, #64]
    mov     x0, x19

    ldp     x19, x20, [sp, #16]
    ldp     x21, x22, [sp, #32]
    ldp     x23, x24, [sp, #48]
    ldp     x25, x26, [sp, #64]
    ldp     x27, x28, [sp, #80]
    ldp     x29, x30, [sp], #96
    ret

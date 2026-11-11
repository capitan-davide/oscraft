/*
 * Kernel thread context switch.
 *
 * void cpu_switch(Context *old, const Context *new, u8 *old_on_cpu,
 *                 u8 *new_on_cpu)
 *
 * Saves the callee-saved registers, lr and sp of the running thread in
 * *old, then clears *old_on_cpu (release): only after that may another CPU
 * resume the old thread. Then waits, with registers only (the old stack
 * may be another CPU's now), until *new_on_cpu is clear (the CPU that ran
 * the new thread last has saved it), sets it, loads *new and returns into
 * it. Everything
 * else is caller-saved by the AAPCS64 call that got us here; the kernel
 * uses no FP/SIMD registers (softfloat target).
 *
 * Context layout: x19..x28, x29, x30, sp, TPIDR_EL0 (14 x 8 bytes). The
 * user thread pointer is switched here because the kernel never uses it.
 */

    .text
    // ANCHOR: cpu_switch
    .global cpu_switch
cpu_switch:
    stp     x19, x20, [x0, #0]
    stp     x21, x22, [x0, #16]
    stp     x23, x24, [x0, #32]
    stp     x25, x26, [x0, #48]
    stp     x27, x28, [x0, #64]
    stp     x29, x30, [x0, #80]
    mov     x9, sp
    mrs     x10, tpidr_el0
    stp     x9, x10, [x0, #96]
    stlrb   wzr, [x2]

1:  ldarb   w9, [x3]
    cbz     w9, 2f
    yield
    b       1b
2:  mov     w9, #1
    strb    w9, [x3]

    ldp     x19, x20, [x1, #0]
    ldp     x21, x22, [x1, #16]
    ldp     x23, x24, [x1, #32]
    ldp     x25, x26, [x1, #48]
    ldp     x27, x28, [x1, #64]
    ldp     x29, x30, [x1, #80]
    ldp     x9, x10, [x1, #96]
    msr     tpidr_el0, x10
    mov     sp, x9
    ret
    // ANCHOR_END: cpu_switch

/*
 * First code of a new user thread: x19 = its TrapFrame at the top of its
 * kernel stack; "return" from it to EL0 (IRQs stay masked until eret).
 */
    .global thread_start_user
thread_start_user:
    mov     sp, x19
    mov     x0, x19
    bl      thread_user_entry       /* kernel work before the first return */
    b       exception_return

/*
 * First code of a new thread: cpu_switch "returns" here with x19 = entry,
 * x20 = argument (set up by Context::new_thread).
 */
    .global thread_start
thread_start:
    mov     x0, x19
    mov     x1, x20
    bl      thread_entry            /* does not return */
1:  wfe
    b       1b

/*
 * FP/SIMD state of a user thread: v0..v31, then FPCR and FPSR (528 bytes).
 * The kernel itself never uses these registers (softfloat target), so a
 * user thread's values survive kernel entry; they are only switched here.
 *
 * void fpsimd_save(FpState *) / void fpsimd_restore(const FpState *)
 */
    .arch_extension fp
    .arch_extension simd

    .global fpsimd_save
fpsimd_save:
    stp     q0, q1, [x0, #0]
    stp     q2, q3, [x0, #32]
    stp     q4, q5, [x0, #64]
    stp     q6, q7, [x0, #96]
    stp     q8, q9, [x0, #128]
    stp     q10, q11, [x0, #160]
    stp     q12, q13, [x0, #192]
    stp     q14, q15, [x0, #224]
    stp     q16, q17, [x0, #256]
    stp     q18, q19, [x0, #288]
    stp     q20, q21, [x0, #320]
    stp     q22, q23, [x0, #352]
    stp     q24, q25, [x0, #384]
    stp     q26, q27, [x0, #416]
    stp     q28, q29, [x0, #448]
    stp     q30, q31, [x0, #480]
    mrs     x1, fpcr
    mrs     x2, fpsr
    add     x3, x0, #512
    stp     x1, x2, [x3]
    ret

    .global fpsimd_restore
fpsimd_restore:
    ldp     q0, q1, [x0, #0]
    ldp     q2, q3, [x0, #32]
    ldp     q4, q5, [x0, #64]
    ldp     q6, q7, [x0, #96]
    ldp     q8, q9, [x0, #128]
    ldp     q10, q11, [x0, #160]
    ldp     q12, q13, [x0, #192]
    ldp     q14, q15, [x0, #224]
    ldp     q16, q17, [x0, #256]
    ldp     q18, q19, [x0, #288]
    ldp     q20, q21, [x0, #320]
    ldp     q22, q23, [x0, #352]
    ldp     q24, q25, [x0, #384]
    ldp     q26, q27, [x0, #416]
    ldp     q28, q29, [x0, #448]
    ldp     q30, q31, [x0, #480]
    add     x3, x0, #512
    ldp     x1, x2, [x3]
    msr     fpcr, x1
    msr     fpsr, x2
    ret

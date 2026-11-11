/*
 * Kernel entry point.
 *
 * The kernel is a bootstrap executable (QNX OS for Safety Startup
 * Specification, appendix A). The startup program enters it on CPU 0 in
 * EL1, with the MMU on (startup's page tables) and x0 = system page
 * (kernel virtual address); another bootstrap executable before it (QNX's
 * kdumper) enters it the same way, x1 then holding the entry address, so
 * only CPU 0 comes here and x1 is not used. The other CPUs wait in
 * startup's smp_spin callout until the kernel releases them.
 */

    .equ    IFS_BOOTSTRAP_SIGNATURE, 0x42534844     /* 'BSHD' */
    .equ    BOOTSTRAP_DATA_SIZE, 0x20
    .equ    BOOT_ARGS_SIZE, 1024

    .section .text.boot, "ax"

    /* struct ifs_bootstrap_head, immediately before the entry point. */
    .balign 8
    .word   IFS_BOOTSTRAP_SIGNATURE
    .word   0
    .quad   bootstrap_data

    .global _start
_start:
    mov     x19, x0                 /* system page */
    mov     x20, #0                 /* CPU 0 */

    mrs     x2, CurrentEL
    cmp     x2, #(1 << 2)
    b.ne    park                    /* startup leaves us in EL1 */

    msr     spsel, #1               /* EL1 code runs on SP_EL1 */
    adrp    x2, __stack_top
    add     x2, x2, :lo12:__stack_top
    mov     sp, x2

    /* Zero .bss (16-byte aligned and sized, see kernel.ld). */
    adrp    x2, __bss_start
    add     x2, x2, :lo12:__bss_start
    adrp    x3, __bss_end
    add     x3, x3, :lo12:__bss_end
1:  cmp     x2, x3
    b.hs    2f
    stp     xzr, xzr, [x2], #16
    b       1b

2:  mov     x0, x19
    mov     x1, x20
    bl      kmain                   /* does not return */

park:
    wfe
    b       park

    /*
     * Entry of CPUs 1..n, released from startup's smp_spin callout by
     * smp::start_secondaries: x0 = system page, x1 = CPU number, MMU on,
     * still on startup's stack. AP_STACK_TOP holds our kernel stack.
     */
    .global ap_start
ap_start:
    msr     spsel, #1
    adrp    x2, AP_STACK_TOP
    ldr     x2, [x2, :lo12:AP_STACK_TOP]
    mov     sp, x2
    bl      kmain_ap                /* does not return */
    b       park

    /*
     * struct ifs_bootstrap_data. mkifs writes our struct bootargs_entry
     * (argv, environment, startup header address) into boot_args, which is
     * in .data so that zeroing .bss does not erase it.
     */
    .section .data.bootstrap, "aw"
    .balign 8
bootstrap_data:
    .word   BOOTSTRAP_DATA_SIZE
    .word   0
    .quad   0                       /* next_entry: unused for the kernel */
    .quad   boot_args
    .word   0
    .word   0

    .global boot_args
    .balign 8
boot_args:
    .space  BOOT_ARGS_SIZE

/*
 * Kernel image layout.
 *
 * The kernel is a bootstrap executable (BSE): mkifs places it in the boot
 * image and the startup program maps its segments, in place, at these
 * virtual addresses (QNX bootstrap space, from vboot = 0xffffff8060000000)
 * before jumping to _start with the MMU on. Each segment starts on its own
 * page so that startup can map it with its own permissions.
 *
 * Two segments, as in procnto: QNX's kdumper reserves exactly two program
 * headers for the kernel's segments in a crash dump (and writes one per
 * PT_LOAD), so a third one pushes the dump's note out of its header table.
 * Read-only data goes with the code (read-only, though executable), not
 * with the data as QNX's link does: it stays read-only.
 */
ENTRY(_start)

KERNEL_BASE = 0xffffff8060100000;
STACK_SIZE = 64K;

PHDRS
{
    text PT_LOAD FLAGS(5);      /* R X: code and read-only data */
    data PT_LOAD FLAGS(6);      /* R W */
}

SECTIONS
{
    . = KERNEL_BASE;
    __kernel_start = .;

    .text : {
        KEEP(*(.text.boot))
        *(.text .text.*)
    } :text

    .rodata : {
        *(.rodata .rodata.*)
    } :text

    . = ALIGN(4K);
    __ro_end = .;               /* end of code and read-only data */
    .data : {
        *(.data .data.*)
    } :data

    .bss (NOLOAD) : ALIGN(16) {
        __bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        . = ALIGN(16);
        __bss_end = .;
    } :data

    /* CPU 0's boot stack, above a page the kernel unmaps at boot (its
     * guard: an overflow faults instead of running into .bss). */
    .stack (NOLOAD) : ALIGN(4K) {
        __stack_guard = .;
        . += 4K;
        . += STACK_SIZE;
        __stack_top = .;
    } :data

    __kernel_end = .;

    /DISCARD/ : {
        *(.comment)
        *(.eh_frame*)
    }
}

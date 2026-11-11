/*
 * Partial link (ld -r) of a startup program into one relocatable object,
 * the form mkifs expects: mkifs does the final link, at the address the
 * image puts startup at. This script only merges the compiler's
 * per-function sections into the usual ones, with _start first.
 */
SECTIONS
{
    .text : {
        *(.text._start)
        *(.text .text.*)
    }
    .rodata : { *(.rodata .rodata.*) }
    .data : { *(.data .data.*) }
    .bss : { *(.bss .bss.*) *(COMMON) }
}

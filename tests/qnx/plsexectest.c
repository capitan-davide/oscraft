/* A new image's process local storage after exec() (as procnto): the
 * kernel's CPUPAGE_PLS word is 0 when the new image starts, before its
 * libc registers its own. The old image's pointer would point into
 * memory that the exec unmapped, and the loader follows it.
 * Run from a shell, which forks and execs it: the shell's libc set its
 * pointer, which the fork copies. The entry point reads the word before
 * crt1's _start runs; built static, with `-Wl,-e,plsexec_start`.
 * Prints "plsexectest: ok"/"FAIL" and "plsexectest: passed". */
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>

uintptr_t pls_at_start = 1;

/* x0 (the loader's function) and sp (argc) go on to crt1's _start. */
__asm__(
    ".globl plsexec_start\n"
    "plsexec_start:\n"
    "    mov x19, x0\n"
    "    mov w0, #1\n"                 /* CPUPAGE_PLS */
    "    bl __SysCpupageGet\n"
    "    adrp x1, pls_at_start\n"
    "    str x0, [x1, :lo12:pls_at_start]\n"
    "    mov x0, x19\n"
    "    b _start\n");

int main(void)
{
    int ok = pls_at_start == 0;
    printf("plsexectest: %s a new image starts with no process local storage (%#lx)\n",
           ok ? "ok" : "FAIL", (unsigned long)pls_at_start);
    printf("plsexectest: %s\n", ok ? "passed" : "FAILED");
    return !ok;
}

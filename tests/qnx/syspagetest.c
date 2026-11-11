/* The system page as a user program sees it (review item CORE-30): libc's
 * _syspage_ptr is a user address, the one the startup chose and procnto
 * maps the page at (system_private's user_syspageptr, in the four pages
 * below the end of the address space), and its bytes can be handed to a
 * server: a write() straight
 * from the strings section reaches the console, a write() of the whole
 * page reaches /dev/null. The kernel used to answer its own kernel
 * address, readable in place but not by a server reading the client's
 * buffer. Prints one "ok"/"FAIL" line per check and "syspagetest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syspage.h>
#include <signal.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("syspagetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    // ANCHOR: syspage_where
    uintptr_t at = (uintptr_t)_syspage_ptr;
    char what[120];
    snprintf(what, sizeof what, "_syspage_ptr %p is a user address", (void *)at);
    check(what, at < (1ULL << 39));
    /* procnto maps the page where the startup said (vm_as_create_default):
     * 0x7fffffd000 on QEMU's virt with OsCraft's startup */
    uintptr_t said = (uintptr_t)SYSPAGE_ENTRY(system_private)->user_syspageptr;
    snprintf(what, sizeof what, "_syspage_ptr is where the startup put it, %p, in the last four pages", (void *)said);
    check(what, at == said && at >= (1ULL << 39) - 4 * 4096);
    // ANCHOR_END: syspage_where
    check("the page says its size", _syspage_ptr->total_size > 0 && _syspage_ptr->total_size < 65536);
    const char *s = SYSPAGE_ENTRY(strings)->data;
    /* the startup's first string (OsCraft's startup, under both kernels) */
    check("the strings section starts with \"memory\"", strcmp(s, "memory") == 0);
    /* the first asinfo entry's name, written from the page itself */
    struct asinfo_entry *as = SYSPAGE_ENTRY(asinfo);
    const char *name = s + as[0].name;
    check("asinfo 0 is \"memory\"", strcmp(name, "memory") == 0);
    errno = 0;
    long n = write(1, "syspagetest: written from the page: ", 36);
    n = write(1, name, strlen(name));
    int e = errno;
    write(1, "\n", 1);
    snprintf(what, sizeof what, "write() of 6 bytes from the page -> %ld errno %d", n, e);
    check(what, n == 6);
    int nul = open("/dev/null", O_WRONLY);
    errno = 0;
    n = write(nul, (const void *)_syspage_ptr, _syspage_ptr->total_size);
    snprintf(what, sizeof what, "write(/dev/null) of the whole page -> %ld errno %d", n, errno);
    check(what, n == _syspage_ptr->total_size);
    close(nul);
    /* read-only: a write to it faults, which is what a child checks */
    pid_t pid = fork();
    if (pid == 0) {
        volatile char *c = (volatile char *)_syspage_ptr;
        *c = *c;
        _exit(0);
    }
    int ws = 0;
    waitpid(pid, &ws, 0);
    check("a write to the page kills the writer (SIGSEGV)", WIFSIGNALED(ws) && WTERMSIG(ws) == SIGSEGV);
    printf("syspagetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

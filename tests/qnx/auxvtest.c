/* The auxiliary vector a program starts with is procnto's (review item
 * PROCMGR-13, measured): the types in its order, no AT_UID/AT_EUID/AT_GID/
 * AT_EGID (the kernel used to write zeros there, whatever the
 * credentials), the dynamic loader's st_dev and st_ino as AT_INTP_DEVICE
 * and AT_INTP_INODE, and AT_FREE_STACK, the stack's size less one page
 * (a 64 KiB stack too). Prints one "ok"/"FAIL" line per check and
 * "auxvtest: passed". */
#include <process.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/auxv.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("auxvtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long get(long t, int *found)
{
    for (auxv_t *a = _auxv; a && a->a_type != AT_NULL; a++) {
        if (a->a_type == t) {
            *found = 1;
            return (long)a->a_un.a_val;
        }
    }
    *found = 0;
    return 0;
}

int main(int argc, char **argv)
{
    int f;
    if (argc > 1 && strcmp(argv[1], "stack") == 0) {
        long free_stack = get(AT_FREE_STACK, &f);
        return f && free_stack == 65536 - 4096 ? 0 : 1;
    }
    /* the types, in order */
    static const long want[] = { AT_PHDR, AT_PHENT, AT_PHNUM, AT_ENTRY, AT_PAGESZ, AT_BASE,
                                 AT_INTP_DEVICE, AT_INTP_INODE, AT_EXEFILE, AT_RANDOM, AT_FREE_STACK };
    int n = 0, same = 1;
    char seen[200] = "";
    for (auxv_t *a = _auxv; a && a->a_type != AT_NULL; a++, n++) {
        char b[8];
        snprintf(b, sizeof b, " %ld", (long)a->a_type);
        strncat(seen, b, sizeof seen - strlen(seen) - 1);
        same &= n < (int)(sizeof want / sizeof *want) && a->a_type == want[n];
    }
    char what[260];
    snprintf(what, sizeof what, "the types, in order:%s", seen);
    check(what, same && n == (int)(sizeof want / sizeof *want));
    get(AT_UID, &f);
    int any = f;
    get(AT_EUID, &f); any |= f;
    get(AT_GID, &f); any |= f;
    get(AT_EGID, &f); any |= f;
    check("no AT_UID, AT_EUID, AT_GID, AT_EGID", !any);
    struct stat st;
    stat("/usr/lib/ldqnx-64.so.2", &st);
    int fd, fi;
    long dev = get(AT_INTP_DEVICE, &fd), ino = get(AT_INTP_INODE, &fi);
    snprintf(what, sizeof what, "AT_INTP_DEVICE %ld, AT_INTP_INODE %ld: the dynamic loader's st_dev %ld, st_ino %ld",
             dev, ino, (long)st.st_dev, (long)st.st_ino);
    check(what, fd && fi && dev == (long)st.st_dev && ino == (long)st.st_ino);
    long free_stack = get(AT_FREE_STACK, &f);
    snprintf(what, sizeof what, "AT_FREE_STACK %ld: the default 512 KiB stack less a page", free_stack);
    check(what, f && free_stack == 524288 - 4096);
    get(AT_PAGESZ, &f);
    check("AT_PAGESZ 4096", f && get(AT_PAGESZ, &f) == 4096);
    posix_spawnattr_t a;
    posix_spawnattr_init(&a);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETSTACKMAX);
    posix_spawnattr_setstackmax(&a, 65536);
    char *args[] = { "auxvtest", "stack", NULL };
    pid_t p;
    int st2 = -1;
    if (posix_spawn(&p, "/proc/boot/auxvtest", NULL, &a, args, NULL) == 0) {
        waitpid(p, &st2, 0);
    }
    check("a 64 KiB stack: AT_FREE_STACK 61440", WIFEXITED(st2) && WEXITSTATUS(st2) == 0);
    printf("auxvtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

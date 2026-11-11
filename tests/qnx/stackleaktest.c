/* A spawn whose main stack is larger than the free memory (review item
 * PROCMGR-14): whatever the kernel answers (procnto backs a stack as it
 * grows and starts the program; OsCraft backs it at once and fails the
 * spawn with ENOMEM), no memory is lost: free memory is the same before
 * and after two such spawns (each zeroes most of memory before it
 * fails, slow on a dev kernel; taken after a first spawn, which costs the
 * kernel a little memory once, and waiting for the failed children's
 * teardown). The kernel used to leak the stack pages it had mapped before
 * running out: all of memory. Prints one "ok"/"FAIL" line per check
 * and "stackleaktest: passed". */
#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("stackleaktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* pidin info's FreeMem, in MiB */
static long freemem(void)
{
    FILE *f = popen("pidin info", "r");
    char line[256];
    long mb = -1;
    while (f && fgets(line, sizeof line, f)) {
        char *p = strstr(line, "FreeMem:");
        if (p) {
            mb = strtol(p + 8, &p, 10);
            if (p && *p == 'G') mb *= 1024;
            if (p && *p == 'K') mb /= 1024;
        }
    }
    if (f) pclose(f);
    return mb;
}

int main(int argc, char **argv)
{
    if (argc > 1) {
        return 0;
    }
    /* a first spawn costs the kernel some memory once (its own tables
     * growing): the baseline is taken after one, once things settle */
    posix_spawnattr_t w;
    posix_spawnattr_init(&w);
    char *wargs[] = { "stackleaktest", "child", NULL };
    pid_t wp;
    if (posix_spawn(&wp, "/proc/boot/stackleaktest", NULL, &w, wargs, NULL) == 0) {
        waitpid(wp, NULL, 0);
    }
    posix_spawnattr_destroy(&w);
    sleep(3);
    long before = freemem();
    char what[160];
    snprintf(what, sizeof what, "free memory read: %ld MiB", before);
    check(what, before > 64);
    int ran = 0, enomem = 0, other = 0;
    for (int i = 0; i < 2; i++) {
        posix_spawnattr_t a;
        posix_spawnattr_init(&a);
        posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETSTACKMAX);
        posix_spawnattr_setstackmax(&a, (size_t)(before + 256) << 20);
        char *args[] = { "stackleaktest", "child", NULL };
        pid_t p;
        int r = posix_spawn(&p, "/proc/boot/stackleaktest", NULL, &a, args, NULL);
        if (r == 0) {
            waitpid(p, NULL, 0);
            ran++;
        } else if (r == ENOMEM) {
            enomem++;
        } else {
            other++;
        }
        posix_spawnattr_destroy(&a);
    }
    /* the failed children's teardown happens in procnto's own time */
    long after = -1;
    for (int t = 0; t < 8 && after < before - 2; t++) {
        sleep(1);
        after = freemem();
    }
    snprintf(what, sizeof what, "2 spawns with a stack of %ld MiB (ran %d, ENOMEM %d, other %d): free %ld -> %ld MiB",
             before + 256, ran, enomem, other, before, after);
    check(what, other == 0 && after >= before - 2);
    printf("stackleaktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

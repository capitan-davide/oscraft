/* Pid numbering, as procnto: a pid is a process-table slot (its low 12
 * bits), the lowest free one, plus a generation that grows by 0x1000 each
 * time a process is waited for; a zombie keeps its slot, and an old pid
 * doesn't name the process that reuses its slot. Boot services may still
 * be starting, so a slot taken by another process is allowed for.
 * Prints one "ok"/"FAIL" line per check and "pidtest: passed". */
#include <dirent.h>
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/wait.h>
#include <unistd.h>

#define SLOT(pid) ((pid) & 0xfff)
#define GEN(pid) ((pid) & ~0xfff)

static int fails;

static void check(const char *what, int ok)
{
    printf("pidtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* A child that exits at once (stay 0) or waits to be killed. */
static pid_t child(int stay)
{
    pid_t p = fork();
    if (p == 0) {
        if (stay) {
            pause();
        }
        _exit(0);
    }
    return p;
}

static void reap(pid_t p, int sig)
{
    if (sig) {
        kill(p, sig);
    }
    waitpid(p, NULL, 0);
}

/* Whether a process other than `except` has slot `slot` now. */
static int slot_taken(int slot, pid_t except)
{
    int taken = 0;
    DIR *d = opendir("/proc");
    struct dirent *e;
    while (d && (e = readdir(d))) {
        pid_t p = atoi(e->d_name);
        taken |= p > 0 && p != except && SLOT(p) == slot;
    }
    if (d) {
        closedir(d);
    }
    return taken;
}

int main(void)
{
    pid_t me = getpid();
    printf("pidtest:   pid %#x\n", me);
    check("a pid has a slot and is positive", me > 0 && SLOT(me) != 0 && SLOT(getppid()) != 0);

    pid_t a = child(0);
    reap(a, 0);
    pid_t b = child(0);
    reap(b, 0);
    printf("pidtest:   sequential %#x %#x\n", a, b);
    check("waiting for a process moves the generation on", GEN(b) > GEN(a));

    pid_t h = child(1);
    reap(h, SIGKILL);
    pid_t c = child(1);
    printf("pidtest:   freed %#x, next %#x\n", h, c);
    check("the lowest free slot", SLOT(c) == SLOT(h) || (SLOT(c) > SLOT(h) && slot_taken(SLOT(h), c)));
    check("a new generation", GEN(c) > GEN(h));
    check("the old pid names no process", SLOT(c) != SLOT(h) || (kill(h, 0) == -1 && errno == ESRCH && kill(c, 0) == 0));
    reap(c, SIGKILL);

    pid_t z = child(0);
    usleep(100000);
    pid_t y = child(1);
    printf("pidtest:   zombie %#x, new %#x\n", z, y);
    check("a zombie keeps its slot", SLOT(y) != SLOT(z));
    reap(z, 0);
    pid_t x = child(1);
    printf("pidtest:   reaped, new %#x\n", x);
    check("then it is free, in a new generation", x != z && GEN(x) > GEN(z));
    reap(x, SIGKILL);
    reap(y, SIGKILL);

    printf("pidtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

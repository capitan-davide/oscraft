/* setpgid()'s rules, as procnto answers them (measured; review item
 * PROCMGR-10): a forked child may be moved to its own group or the
 * caller's, not to a group that doesn't exist (EPERM); a child that runs
 * a program of its own, spawned or forked and exec'ed, can't be moved
 * (EACCES; the kernel used to move it); a child in another session
 * (EPERM), a process that isn't a child (ESRCH), a session leader
 * (EPERM), a negative group (EINVAL). The test runs in a session of its
 * own. Prints one "ok"/"FAIL" line per check and "setpgidtest: passed". */
#include <errno.h>
#include <process.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define SELF "/proc/boot/setpgidtest"
static int fails;

static void check(const char *what, int r, int err)
{
    int ok = err ? r == -1 && errno == err : r == 0;
    printf("setpgidtest: %s %s (%d, errno %d)\n", ok ? "ok" : "FAIL", what, r, r < 0 ? errno : 0);
    fflush(stdout);
    fails += !ok;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "sleeper") == 0) {
        sleep(5);
        return 0;
    }
    if (argc == 1) {
        /* the checks run in a session of their own */
        pid_t p = fork();
        if (p == 0) {
            setsid();
            execl(SELF, "setpgidtest", "test", NULL);
            _exit(1);
        }
        int st = 0;
        waitpid(p, &st, 0);
        return WIFEXITED(st) ? WEXITSTATUS(st) : 1;
    }
    pid_t c1 = fork();
    if (c1 == 0) { sleep(5); _exit(0); }
    usleep(20000);
    errno = 0; check("forked child: to its own group", setpgid(c1, c1), 0);
    errno = 0; check("forked child: pgid 0", setpgid(c1, 0), 0);
    errno = 0; check("forked child: to the caller's group", setpgid(c1, getpgrp()), 0);
    errno = 0; check("forked child: to a group that doesn't exist", setpgid(c1, 99999), EPERM);
    errno = 0; check("forked child: a negative group", setpgid(c1, -1), EINVAL);
    pid_t c2 = spawnl(P_NOWAIT, SELF, "setpgidtest", "sleeper", NULL);
    usleep(50000);
    errno = 0; check("spawned child", setpgid(c2, c2), EACCES);
    pid_t c3 = fork();
    if (c3 == 0) { execl(SELF, "setpgidtest", "sleeper", NULL); _exit(1); }
    usleep(100000);
    errno = 0; check("forked child after exec", setpgid(c3, c3), EACCES);
    pid_t c4 = fork();
    if (c4 == 0) { setsid(); sleep(5); _exit(0); }
    usleep(50000);
    errno = 0; check("a child in a session of its own", setpgid(c4, c4), EPERM);
    errno = 0; check("not a child (pid 1)", setpgid(1, 1), ESRCH);
    errno = 0; check("no such process", setpgid(99999, 0), ESRCH);
    errno = 0; check("self, a session leader", setpgid(0, 0), EPERM);
    kill(c1, SIGKILL); kill(c2, SIGKILL); kill(c3, SIGKILL); kill(c4, SIGKILL);
    while (wait(NULL) > 0) {
    }
    pid_t c5 = fork();
    if (c5 == 0) {
        errno = 0; check("a child, not a leader: itself to its own group", setpgid(0, 0), 0);
        errno = 0; check("a child: itself to a group of another session", setpgid(0, 1), EPERM);
        _exit(fails);
    }
    int st = 0;
    waitpid(c5, &st, 0);
    fails += !WIFEXITED(st) || WEXITSTATUS(st) != 0;
    printf("setpgidtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

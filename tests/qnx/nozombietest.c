/* An exec that severs the program from its parent (POSIX_SPAWN_EXEC |
 * POSIX_SPAWN_NOZOMBIE, procnto's procmgr_nozombie; review item
 * PROCMGR-21), as procnto (measured): the parent's blocking waitpid()
 * returns at once with the child's pid and exit status 0, the parent gets
 * a SIGCHLD, and the program runs on (it writes to a pipe 300 ms later).
 * The waits it settles are now answered by procnto's system-manager
 * thread only. Prints one "ok"/"FAIL" line per check and
 * "nozombietest: passed". */
#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;
static volatile int sigchld;

static void check(const char *what, int ok)
{
    printf("nozombietest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void on_chld(int s)
{
    (void)s;
    sigchld++;
}

static uint64_t now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000ULL + ts.tv_nsec / 1000000;
}

int main(int argc, char **argv)
{
    if (argc > 2 && strcmp(argv[1], "run") == 0) {
        /* the severed program: tell the test it runs on */
        int fd = argv[2][0] - '0';
        usleep(300000);
        write(fd, "alive", 5);
        return 0;
    }
    signal(SIGCHLD, on_chld);
    int p[2];
    pipe(p);
    pid_t c = fork();
    if (c == 0) {
        close(p[0]);
        char fd[2] = { (char)('0' + p[1]), 0 };
        char *args[] = { "nozombietest", "run", fd, NULL };
        posix_spawnattr_t a;
        posix_spawnattr_init(&a);
        posix_spawnattr_setxflags(&a, POSIX_SPAWN_EXEC | POSIX_SPAWN_NOZOMBIE);
        pid_t ignored;
        int r = posix_spawn(&ignored, "/proc/boot/nozombietest", NULL, &a, args, NULL);
        printf("nozombietest: FAIL posix_spawn(EXEC | NOZOMBIE): %d\n", r);
        _exit(1);
    }
    close(p[1]);
    uint64_t t0 = now_ms();
    int st = -1;
    pid_t w = waitpid(c, &st, 0);
    uint64_t waited = now_ms() - t0;
    check("waitpid() returns the child", w == c);
    check("with exit status 0", WIFEXITED(st) && WEXITSTATUS(st) == 0);
    check("at once, before the program ends", waited < 250);
    usleep(10000);
    if (sigchld != 1)
        printf("nozombietest: %d SIGCHLDs\n", sigchld);
    check("the parent got one SIGCHLD", sigchld == 1);
    char buf[8] = { 0 };
    int n = read(p[0], buf, sizeof buf);
    check("the program ran on", n == 5 && memcmp(buf, "alive", 5) == 0);
    errno = 0;
    check("and there is nothing more to wait for", waitpid(-1, &st, WNOHANG) == -1 && errno == ECHILD);
    printf("nozombietest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

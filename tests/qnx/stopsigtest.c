/* SIGTSTP, SIGTTIN and SIGTTOU (review item SCHED-28), as procnto
 * (measured): at their default action they stop the process, as SIGSTOP
 * does, the signal its stop signal (waitpid(WUNTRACED): WSTOPSIG), and
 * SIGCONT lets it go on; a handler catches SIGTSTP, and SIG_IGN ignores
 * it; a blocked SIGTSTP stops the process when it is unblocked. A
 * process in an orphaned process group stops too: procnto doesn't apply
 * POSIX's rule that discards them there, for kill() (measured, with a
 * setsid() child and a grandchild whose parent has ended). The kernel
 * ignored them by default. Each case in a child. Prints
 * one "ok"/"FAIL" line per check and "stopsigtest: passed". */
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static volatile int got;

static void check(const char *what, int ok)
{
    printf("stopsigtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void h(int s)
{
    got = s;
}

/* the child: runs for a second, exits 7 if its handler ran */
static void child(int mode)
{
    if (mode == 1)
        signal(SIGTSTP, h);
    if (mode == 2)
        signal(SIGTSTP, SIG_IGN);
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGTSTP);
    if (mode == 3)
        sigprocmask(SIG_BLOCK, &set, NULL);
    for (int i = 0; i < 100; i++) {
        usleep(10000);
        if (mode == 3 && i == 20)
            sigprocmask(SIG_UNBLOCK, &set, NULL); /* 200 ms in */
        if (got)
            _exit(7);
    }
    _exit(0);
}

/* send `sig` to a child (`mode`: 0 default, 1 handler, 2 ignored, 3
 * blocked for its first 200 ms); after `ms`, the
 * outcome: 's' stopped (by `*stopsig`), then continued and exited 0, or
 * the exit status */
static int run(int sig, int mode, int *stopsig, int ms)
{
    pid_t c = fork();
    if (c == 0)
        child(mode);
    usleep(50000);
    kill(c, sig);
    usleep(ms * 1000);
    int st = 0;
    pid_t w = waitpid(c, &st, WUNTRACED | WNOHANG);
    if (w == c && WIFSTOPPED(st)) {
        *stopsig = WSTOPSIG(st);
        kill(c, SIGCONT);
        w = waitpid(c, &st, WCONTINUED);
        int cont = w == c && WIFCONTINUED(st);
        waitpid(c, &st, 0);
        return cont && WIFEXITED(st) && WEXITSTATUS(st) == 0 ? 's' : -1;
    }
    waitpid(c, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : -1;
}

/* SIGTSTP sent while the child blocks it: pending, then it stops the
 * child when the child unblocks it, 200 ms in */
static void blocked(void)
{
    pid_t c = fork();
    if (c == 0)
        child(3);
    usleep(50000);
    kill(c, SIGTSTP);
    usleep(50000);
    int st = 0;
    check("SIGTSTP blocked: the process runs on", waitpid(c, &st, WUNTRACED | WNOHANG) == 0);
    usleep(250000);
    pid_t w = waitpid(c, &st, WUNTRACED | WNOHANG);
    check("SIGTSTP unblocked: then it stops", w == c && WIFSTOPPED(st) && WSTOPSIG(st) == SIGTSTP);
    kill(c, SIGCONT);
    waitpid(c, &st, 0);
    check("and SIGCONT lets it end", WIFEXITED(st) && WEXITSTATUS(st) == 0);
}

/* A child in an orphaned process group, by setsid() (its parent, this
 * process, is in another session): `sig` at its default action stops it
 * all the same (procnto, measured: no orphan rule for kill()). */
static int orphan_setsid(int sig)
{
    int p[2];
    pipe(p);
    pid_t c = fork();
    if (c == 0) {
        setsid();
        write(p[1], "x", 1);
        for (int i = 0; i < 100; i++)
            usleep(10000);
        _exit(0);
    }
    char b;
    read(p[0], &b, 1);
    usleep(20000);
    kill(c, sig);
    usleep(50000);
    int st = 0;
    int stopped = waitpid(c, &st, WUNTRACED | WNOHANG) == c && WIFSTOPPED(st) && WSTOPSIG(st) == sig;
    kill(c, SIGCONT);
    waitpid(c, &st, 0);
    close(p[0]);
    close(p[1]);
    return stopped;
}

/* A grandchild in its own process group whose parent has ended (an
 * orphaned group, procnto its parent): it writes a byte every 10 ms;
 * after `sig`, does it go quiet (stopped)? */
static int orphan_gone(int sig)
{
    int p[2], q[2];
    pipe(p);
    pipe(q);
    pid_t c = fork();
    if (c == 0) {
        pid_t g = fork();
        if (g == 0) {
            setpgid(0, 0);
            for (int i = 0; i < 300; i++) {
                write(p[1], "x", 1);
                usleep(10000);
            }
            _exit(0);
        }
        write(q[1], &g, sizeof g);
        _exit(0);
    }
    pid_t g;
    read(q[0], &g, sizeof g);
    waitpid(c, NULL, 0);
    usleep(50000);
    kill(g, sig);
    usleep(50000);
    fcntl(p[0], F_SETFL, O_NONBLOCK);
    char buf[512];
    while (read(p[0], buf, sizeof buf) > 0) {
    }
    usleep(100000);
    int quiet = read(p[0], buf, sizeof buf) <= 0;
    kill(g, SIGKILL);
    close(p[0]);
    close(p[1]);
    close(q[0]);
    close(q[1]);
    return quiet;
}

int main(void)
{
    static const struct {
        int sig;
        const char *name;
    } stops[] = { { SIGTSTP, "SIGTSTP" }, { SIGTTIN, "SIGTTIN" }, { SIGTTOU, "SIGTTOU" }, { SIGSTOP, "SIGSTOP" } };
    for (unsigned i = 0; i < sizeof stops / sizeof *stops; i++) {
        int by = 0;
        char what[96];
        int r = run(stops[i].sig, 0, &by, 50);
        snprintf(what, sizeof what, "%s stops the process, SIGCONT lets it go on", stops[i].name);
        check(what, r == 's');
        snprintf(what, sizeof what, "%s is its stop signal", stops[i].name);
        check(what, by == stops[i].sig);
    }
    int by = 0;
    check("SIGTSTP caught by a handler", run(SIGTSTP, 1, &by, 50) == 7);
    check("SIGTSTP ignored", run(SIGTSTP, 2, &by, 50) == 0);
    blocked();
    check("SIGTSTP stops a child in an orphaned group (setsid)", orphan_setsid(SIGTSTP));
    check("SIGTTIN stops a child in an orphaned group (setsid)", orphan_setsid(SIGTTIN));
    check("SIGTTOU stops a child in an orphaned group (setsid)", orphan_setsid(SIGTTOU));
    check("SIGTSTP stops an orphaned grandchild (parent gone)", orphan_gone(SIGTSTP));
    check("SIGTTOU stops an orphaned grandchild (parent gone)", orphan_gone(SIGTTOU));

    printf("stopsigtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

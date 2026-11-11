/* A stop that doesn't come from a process with a side connection still
 * answers the parent's waitpid(WUNTRACED) (review item IPC-15): a child
 * whose timer sends it SIGSTOP (delivered from the clock tick), and a
 * continue by SIGCONT from the parent, as on procnto (measured). A child
 * spawned held (POSIX_SPAWN_HOLD) isn't stopped by a signal: procnto
 * doesn't report it to waitpid(WUNTRACED), and it runs to its end after
 * SIGCONT. Each
 * wait is bounded: a lost notification fails, rather than hanging.
 * Prints one "ok"/"FAIL" line per check and "jobpulsetest: passed". */
#include <errno.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <time.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("jobpulsetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void on_alarm(int s)
{
    (void)s;
}

/* waitpid(p, flags) with a 3 s bound: its result, -1 if it timed out. */
static pid_t wait_bounded(pid_t p, int *st, int flags)
{
    struct sigaction sa = {.sa_handler = on_alarm};
    sigaction(SIGALRM, &sa, NULL);
    alarm(3);
    pid_t r = waitpid(p, st, flags);
    alarm(0);
    return r;
}

int main(int argc, char **argv)
{
    if (argc > 1)
        return 0; /* the held child */
    pid_t p = fork();
    if (p == 0) {
        struct sigevent ev;
        SIGEV_SIGNAL_INIT(&ev, SIGSTOP);
        timer_t t;
        timer_create(CLOCK_MONOTONIC, &ev, &t);
        struct itimerspec its = {.it_value = {0, 50000000}};
        timer_settime(t, 0, &its, NULL);
        for (;;)
            pause();
    }
    int st = 0;
    pid_t r = wait_bounded(p, &st, WUNTRACED);
    check("a timer's SIGSTOP: waitpid(WUNTRACED) answers, stopped by SIGSTOP",
          r == p && WIFSTOPPED(st) && WSTOPSIG(st) == SIGSTOP);
    kill(p, SIGCONT);
    r = wait_bounded(p, &st, WCONTINUED);
    check("SIGCONT: waitpid(WCONTINUED) answers", r == p && WIFCONTINUED(st));
    kill(p, SIGKILL);
    waitpid(p, NULL, 0);

    posix_spawnattr_t a;
    posix_spawnattr_init(&a);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_HOLD);
    char *args[] = {argv[0], "child", NULL};
    char *env[] = {"PATH=/proc/boot:/bin:/usr/bin", NULL};
    int e = posix_spawnp(&p, argv[0], NULL, &a, args, env);
    r = e ? -1 : wait_bounded(p, &st, WUNTRACED);
    check("spawned held: not reported to waitpid(WUNTRACED)", !e && r == -1);
    if (!e) {
        kill(p, SIGCONT);
        r = wait_bounded(p, &st, 0);
        check("and it runs to its end after SIGCONT", r == p && WIFEXITED(st) && WEXITSTATUS(st) == 0);
    }
    printf("jobpulsetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

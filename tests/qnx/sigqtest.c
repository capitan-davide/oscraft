/* Queueing of signals that a thread blocks and takes with sigwaitinfo(),
 * as on procnto:
 * - without SA_SIGINFO, the first one sent stays pending, the others are
 *   dropped (process- and thread-directed);
 * - the top 8 signals (SIGSELECT on, which libc's poll() uses) are always
 *   queued, every one in order;
 * - a pending signal sent to the thread is taken before one sent to the
 *   process, whatever the order they came in;
 * - sigtimedwait() with a zero timeout returns at once (EAGAIN) when
 *   nothing is pending.
 * Prints one "ok"/"FAIL" line per check and "sigqtest: passed". */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("sigqtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* The values of the pending instances of `sig`, in the order taken. */
static int drain(int sig, int *values, int max)
{
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, sig);
    siginfo_t info;
    struct timespec t = {0, 0};
    int n = 0;
    while (n < max && sigtimedwait(&set, &info, &t) == sig) {
        values[n++] = info.si_value.sival_int;
    }
    return n;
}

int main(void)
{
    int v[8], n;
    int sigs[] = {SIGUSR1, SIGRTMIN, SIGSELECT};
    for (int i = 0; i < 3; i++) {
        sigset_t set;
        sigemptyset(&set);
        sigaddset(&set, sigs[i]);
        sigprocmask(SIG_BLOCK, &set, NULL);
        SignalKill(0, getpid(), 0, sigs[i], SI_USER, 100);
        SignalKill(0, getpid(), 0, sigs[i], SI_USER, 200);
        SignalKill(0, getpid(), 0, sigs[i], SI_USER, 300);
        n = drain(sigs[i], v, 8);
        char what[64];
        if (sigs[i] == SIGSELECT) {
            snprintf(what, sizeof what, "SIGSELECT: all three queued, in order");
            check(what, n == 3 && v[0] == 100 && v[1] == 200 && v[2] == 300);
        } else {
            snprintf(what, sizeof what, "signal %d: the first one kept", sigs[i]);
            check(what, n == 1 && v[0] == 100);
        }
        SignalKill(0, getpid(), gettid(), sigs[i], SI_USER, 11);
        SignalKill(0, getpid(), gettid(), sigs[i], SI_USER, 22);
        n = drain(sigs[i], v, 8);
        snprintf(what, sizeof what, "signal %d to the thread", sigs[i]);
        check(what, sigs[i] == SIGSELECT ? n == 2 && v[0] == 11 && v[1] == 22 : n == 1 && v[0] == 11);
    }

    /* Queued (SA_SIGINFO): the process's sent first, the thread's after. */
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = SIG_DFL;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGRTMIN + 1, &sa, NULL);
    sigset_t rt;
    sigemptyset(&rt);
    sigaddset(&rt, SIGRTMIN + 1);
    sigprocmask(SIG_BLOCK, &rt, NULL);
    SignalKill(0, getpid(), 0, SIGRTMIN + 1, SI_USER, 2);
    SignalKill(0, getpid(), gettid(), SIGRTMIN + 1, SI_USER, 1);
    n = drain(SIGRTMIN + 1, v, 8);
    check("the thread's signal is taken before the process's", n == 2 && v[0] == 1 && v[1] == 2);

    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, NULL);
    struct timespec zero = {0, 0}, a, b;
    clock_gettime(CLOCK_MONOTONIC, &a);
    int r = 0, e = 0;
    for (int i = 0; i < 100; i++) {
        errno = 0;
        r = sigtimedwait(&set, NULL, &zero);
        e = errno;
    }
    clock_gettime(CLOCK_MONOTONIC, &b);
    long us = (b.tv_sec - a.tv_sec) * 1000000 + (b.tv_nsec - a.tv_nsec) / 1000;
    check("sigtimedwait() with a zero timeout: EAGAIN at once", r == -1 && e == EAGAIN && us < 50000);
    printf("sigqtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

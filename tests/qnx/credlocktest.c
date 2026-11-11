/* Two processes on two CPUs hammering each other with the two kernel
 * paths that compare their credentials (review item IPC-5): a pulse to
 * the other's channel (MsgSendPulse's cross-process check) and a read of
 * the other's CPU time (ClockTime on its CPU-time clock, through
 * ability::may_query). Before the fix each path took one process's
 * credential lock and then the other's; two processes doing it to each
 * other at once could each hold one and spin on the other with IRQs
 * masked, and the machine stopped. Two seconds of it, then both report.
 * Prints "ok"/"FAIL" lines and "credlocktest: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("credlocktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

/* Pulses `other`'s channel `chid` and reads its CPU time for 2 s, pinned
 * to `cpu`; returns the number of rounds, or -1 on an error. */
static long hammer(pid_t other, int chid, int cpu)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
    int coid = ConnectAttach(0, other, chid, _NTO_SIDE_CHANNEL, 0);
    if (coid < 0) {
        return -1;
    }
    clockid_t id = ClockId(other, 1);
    if (id == (clockid_t)-1) {
        return -1;
    }
    long rounds = 0, until = now_ms() + 2000;
    struct timespec t;
    while (now_ms() < until) {
        if (MsgSendPulse(coid, -1, _PULSE_CODE_MINAVAIL, 0) != 0 && errno != EAGAIN) {
            return -1;
        }
        if (clock_gettime(id, &t) != 0) {
            return -1;
        }
        rounds++;
    }
    ConnectDetach(coid);
    return rounds;
}

/* Drains the pulses sent to `chid` until told to stop. */
static volatile int stop;
static void *drain(void *arg)
{
    int chid = (int)(intptr_t)arg;
    struct _pulse p;
    uint64_t ns = 100000000;
    while (!stop) {
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
        MsgReceivePulse(chid, &p, sizeof p, NULL);
    }
    return NULL;
}

int main(void)
{
    int p2c[2], c2p[2];
    pipe(p2c);
    pipe(c2p);
    pid_t child = fork();
    if (child == 0) {
        int chid = ChannelCreate(0);
        pthread_t d;
        pthread_create(&d, NULL, drain, (void *)(intptr_t)chid);
        write(c2p[1], &chid, sizeof chid);
        int pchid;
        read(p2c[0], &pchid, sizeof pchid);
        long r = hammer(getppid(), pchid, 1);
        /* Until the parent is done with this process's channel and clock:
           it started later, and would find them gone. */
        char done;
        read(p2c[0], &done, 1);
        stop = 1;
        pthread_join(d, NULL);
        _exit(r < 0 ? 1 : 0);
    }
    int chid = ChannelCreate(0);
    pthread_t d;
    pthread_create(&d, NULL, drain, (void *)(intptr_t)chid);
    int cchid;
    read(c2p[0], &cchid, sizeof cchid);
    write(p2c[1], &chid, sizeof chid);
    long r = hammer(child, cchid, 2);
    write(p2c[1], "", 1);
    stop = 1;
    pthread_join(d, NULL);
    int st = -1;
    waitpid(child, &st, 0);
    printf("credlocktest: %ld rounds in the parent\n", r);
    check("the parent's pulses and queries all went through", r > 0);
    check("the child's too, and it ended", WIFEXITED(st) && WEXITSTATUS(st) == 0);
    printf("credlocktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* A long MsgWrite() (issue #287), as procnto (measured, QEMU): a 1 ms
 * SCHED_FIFO thread at 60 on the writer's CPU stays on time while a server
 * at 10 writes 64 MiB replies (procnto: 0.8 ms late at worst; OsCraft wrote
 * each as one masked piece, 130 ms), and the data arrives intact. A client
 * signalled mid-write, on a channel without _NTO_CHF_UNBLOCK, leaves with
 * EINTR, its buffer isn't written after, and the server's MsgWrite() fails
 * with ESRCH (OsCraft finished the write before letting the client go).
 * Prints "msgwritetest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

#define BIG (64 << 20)
#define LIMIT_US 20000
#define QUIT 0x7f

static int chid, fails;
static char *wbuf;
static volatile int stop, writes, wrote_ret, wrote_err, kick_us;
static volatile long long worst;
static pthread_t client;

static void check(const char *what, int ok)
{
    printf("msgwritetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long long ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void pin(int cpu)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
}

static void prio(int p)
{
    struct sched_param sp = { .sched_priority = p };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
}

/* On CPU 1 at 10: answers each message with a 64 MiB MsgWrite(), then an
 * empty reply. */
static void *server(void *a)
{
    pin(1);
    prio(10);
    memset(wbuf, 'W', BIG);
    for (;;) {
        char m;
        int rcvid = MsgReceive(chid, &m, 1, NULL);
        if (rcvid <= 0)
            continue;
        if (m == QUIT) {
            MsgReply(rcvid, 0, NULL, 0);
            return a;
        }
        int r = MsgWrite(rcvid, wbuf, BIG, 0);
        wrote_ret = r;
        wrote_err = r == -1 ? errno : 0;
        writes++;
        MsgReply(rcvid, 0, NULL, 0);
    }
}

/* On CPU 1 at 60: wakes every 1 ms (absolute deadlines), keeps the worst
 * lateness. */
static void *meter(void *a)
{
    pin(1);
    prio(60);
    struct timespec next;
    clock_gettime(CLOCK_MONOTONIC, &next);
    while (!stop) {
        next.tv_nsec += 1000000;
        if (next.tv_nsec >= 1000000000) {
            next.tv_nsec -= 1000000000;
            next.tv_sec++;
        }
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL);
        long long late = ns() - (next.tv_sec * 1000000000LL + next.tv_nsec);
        if (late > worst)
            worst = late;
    }
    return a;
}

static void *kicker(void *a)
{
    pin(3);
    usleep(kick_us);
    pthread_kill(client, SIGUSR1);
    return a;
}

static void on_usr1(int s)
{
    (void)s;
}

static long long sum(const char *b)
{
    long long s = 0;
    for (int j = 0; j < BIG; j += 4096)
        s += b[j];
    return s;
}

int main(void)
{
    wbuf = malloc(BIG);
    char *rbuf = malloc(BIG);
    chid = ChannelCreate(0);
    pthread_t s, m;
    pthread_create(&s, NULL, server, NULL);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    pin(2);

    pthread_create(&m, NULL, meter, NULL);
    int n = 0, intact = 1;
    long long t0 = ns();
    while (ns() - t0 < 2000000000LL) {
        char c = 'a';
        rbuf[0] = rbuf[BIG / 2] = rbuf[BIG - 1] = 0;
        intact &= MsgSend(coid, &c, 1, rbuf, BIG) == 0 && rbuf[0] == 'W' && rbuf[BIG / 2] == 'W' &&
                  rbuf[BIG - 1] == 'W';
        n++;
    }
    stop = 1;
    pthread_join(m, NULL);
    printf("msgwritetest:   %d writes of 64 MiB, worst wakeup %lld us late\n", n, worst / 1000);
    check("each 64 MiB reply arrives intact", n > 0 && intact);
    check("a 1 ms thread at 60 on the writer's CPU stays within 20 ms", worst / 1000 < LIMIT_US);

    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_usr1;
    sigaction(SIGUSR1, &sa, NULL);
    client = pthread_self();
    int eintr = 0, esrch = 0, unchanged = 0;
    for (int i = 0; i < 10; i++) {
        memset(rbuf, '.', BIG);
        int before = writes;
        kick_us = 2000 + i * 500;
        pthread_t k;
        pthread_create(&k, NULL, kicker, NULL);
        char c = 'b';
        int r = MsgSend(coid, &c, 1, rbuf, BIG);
        int e = errno;
        pthread_join(k, NULL);
        long long s1 = sum(rbuf);
        while (writes == before)
            usleep(1000);
        if (r == -1 && e == EINTR) {
            eintr++;
            esrch += wrote_ret == -1 && wrote_err == ESRCH;
            unchanged += sum(rbuf) == s1;
        }
    }
    printf("msgwritetest:   %d of 10 sends interrupted mid-write\n", eintr);
    check("a client signalled mid-write leaves with EINTR", eintr > 0);
    check("the server's MsgWrite() then fails with ESRCH", esrch == eintr);
    check("and the client's buffer isn't written after it left", unchanged == eintr);

    char q = QUIT;
    MsgSend(coid, &q, 1, NULL, 0);
    printf("msgwritetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

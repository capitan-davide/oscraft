/* Registered events and thread-directed timer signals (review items
 * SCHED-25 and SCHED-24), as procnto (measured):
 * - a process may have 256 events registered with MsgRegisterEvent(), over
 *   all its connections and channels; one more is EAGAIN, and
 *   MsgUnregisterEvent() makes room again. The kernel had no limit.
 * - the signal of a SIGEV_SIGNAL_THREAD timer whose thread has ended goes
 *   to thread 1, not to a new thread that has the ended one's ID (which
 *   blocks it here; thread 1 doesn't). The kernel gave it to the new
 *   thread.
 * Prints one "ok"/"FAIL" line per check and "eventregtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

static int fails;
static struct sigevent evs[300];
static volatile int got_in;
static timer_t t;
static int tr;

static void check(const char *what, int ok)
{
    printf("eventregtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void h(int s)
{
    (void)s;
    got_in = gettid();
}

/* makes the timer whose signals are for this thread, then ends */
static void *maker(void *a)
{
    (void)a;
    struct sigevent te;
    SIGEV_SIGNAL_THREAD_INIT(&te, SIGUSR1, 0, 0);
    tr = timer_create(CLOCK_MONOTONIC, &te, &t);
    return NULL;
}

static void *blocker(void *a)
{
    (void)a;
    sigset_t s;
    sigemptyset(&s);
    sigaddset(&s, SIGUSR1);
    pthread_sigmask(SIG_BLOCK, &s, NULL);
    usleep(400000);
    return NULL;
}

static int reg(int coid, int i)
{
    SIGEV_PULSE_INIT(&evs[i], coid, 10, 1, 0);
    return MsgRegisterEvent(&evs[i], coid);
}

int main(void)
{
    int chid = ChannelCreate(0), chid2 = ChannelCreate(0);
    int c1 = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    int c2 = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    int c3 = ConnectAttach(0, 0, chid2, _NTO_SIDE_CHANNEL, 0);
    int n = 0;
    while (n < 256 && reg(n < 128 ? c1 : c2, n) == 0)
        n++;
    check("256 events registered, over two connections", n == 256);
    errno = 0;
    check("the 257th: EAGAIN", reg(c1, 256) == -1 && errno == EAGAIN);
    errno = 0;
    check("on another channel too: EAGAIN", reg(c3, 256) == -1 && errno == EAGAIN);
    check("MsgUnregisterEvent()", MsgUnregisterEvent(&evs[0]) == 0);
    check("then one more fits", reg(c3, 0) == 0);
    for (int i = 0; i < 256; i++)
        MsgUnregisterEvent(&evs[i]);

    signal(SIGUSR1, h);
    pthread_t m, b;
    pthread_create(&m, NULL, maker, NULL);
    pthread_join(m, NULL);
    check("timer_create(SIGEV_SIGNAL_THREAD) in a thread that then ended", tr == 0);
    pthread_create(&b, NULL, blocker, NULL);
    check("a new thread has its ID", b == m);
    usleep(20000);
    struct itimerspec it = { .it_value = { 0, 50000000 } };
    timer_settime(t, 0, &it, NULL);
    usleep(200000);
    check("the timer's signal goes to thread 1", got_in == 1);
    pthread_join(b, NULL);
    printf("eventregtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

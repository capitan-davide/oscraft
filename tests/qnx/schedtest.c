/* Scheduling as procnto does it:
 * - SchedInfo(): priorities 1..253, priority_priv 63, the round-robin
 *   timeslice (3.75 ms: 4 ticks less a quarter) for SCHED_RR and
 *   SCHED_OTHER, 0 for SCHED_FIFO and SCHED_SPORADIC;
 * - SCHED_OTHER stays SCHED_OTHER;
 * - a round-robin thread gives way to an equal one at the first tick after
 *   3.75 ms of CPU time (turns of 4 ms), also when a higher-priority thread
 *   wakes at that tick (it goes behind the other, not back to the head);
 * - SCHED_SPORADIC: the parameters checked (EINVAL), kept (SchedGet()),
 *   and a sporadic thread runs its budget at its normal priority, then at
 *   its low one until the replenishment period has passed;
 * - new threads take the creator's policy, or the attribute's.
 * Prints one "ok"/"FAIL" line per check and "schedtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <time.h>

static int fails;
static volatile int stop;

static void check(const char *what, int ok)
{
    printf("schedtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long now_us(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000L + t.tv_nsec / 1000;
}

static void pin(void)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)1);
}

static void info(void)
{
    struct _sched_info si;
    check("SchedInfo(SCHED_RR)", SchedInfo(0, SCHED_RR, &si) == 0);
    check("priorities 1..253, priority_priv 63",
          si.priority_min == 1 && si.priority_max == 253 && si.priority_priv == 63);
    check("the timeslice: 3.75 ms", si.interval == 3750000);
    SchedInfo(0, SCHED_OTHER, &si);
    check("SCHED_OTHER's too", si.interval == 3750000);
    SchedInfo(0, SCHED_FIFO, &si);
    check("SCHED_FIFO: 0", si.interval == 0);
    check("SchedInfo(SCHED_SPORADIC): 0", SchedInfo(0, SCHED_SPORADIC, &si) == 0 && si.interval == 0);
    errno = 0;
    check("an unknown policy: EINVAL", SchedInfo(0, 99, &si) == -1 && errno == EINVAL);
    struct sched_param sp = { .sched_priority = 10 };
    SchedSet(0, 0, SCHED_OTHER, &sp);
    check("SCHED_OTHER stays SCHED_OTHER", sched_getscheduler(0) == SCHED_OTHER);
}

/* Two round-robin threads of one priority on one CPU: the times at which
 * they take turns. At 50, above the image's services (io-sock's threads
 * run at 21), which would otherwise lengthen a turn now and then. */
static volatile int turn = -1, nturns;
static long turns[24];

static void *rr(void *arg)
{
    int me = (int)(long)arg;
    /* Its priority first: pinned below the other, it wouldn't run. */
    struct sched_param sp = { .sched_priority = 50 };
    SchedSet(0, 0, SCHED_RR, &sp);
    pin();
    while (!stop) {
        if (turn != me) {
            turn = me;
            if (nturns < 24) {
                turns[nturns++] = now_us();
            }
        }
    }
    return NULL;
}

/* A thread above them on their CPU that wakes at every tick (absolute
 * sleeps of 1 ms). */
static void *ticker(void *arg)
{
    (void)arg;
    struct sched_param sp = { .sched_priority = 60 };
    SchedSet(0, 0, SCHED_FIFO, &sp);
    pin();
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    while (!stop) {
        ts.tv_nsec += 1000000;
        if (ts.tv_nsec >= 1000000000) {
            ts.tv_sec++;
            ts.tv_nsec -= 1000000000;
        }
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &ts, NULL);
    }
    return NULL;
}

/* The turns with a thread woken at every tick above them: a turn that
 * ends at a tick still goes to the other thread (on procnto turns of 4 ms,
 * the longest 5 ms). Before, OsCraft put the preempted thread back at the
 * head with a new timeslice, and the other never ran. */
static void woken_every_tick(void)
{
    pthread_t a, b, t;
    stop = 0;
    nturns = 0;
    turn = -1;
    pthread_create(&t, NULL, ticker, NULL);
    pthread_create(&a, NULL, rr, (void *)0);
    pthread_create(&b, NULL, rr, (void *)1);
    struct timespec ts = { 0, 150000000 };
    nanosleep(&ts, NULL);
    stop = 1;
    pthread_join(a, NULL);
    pthread_join(b, NULL);
    pthread_join(t, NULL);
    long longest = 0;
    for (int i = 4; i < nturns; i++) {
        long d = turns[i] - turns[i - 1];
        if (d > longest) {
            longest = d;
        }
    }
    if (nturns != 24 || longest > 6500) {
        printf("schedtest:   %d turns, the longest %ld us\n", nturns, longest);
    }
    check("round-robin turns with a thread woken at every tick above them", nturns == 24 && longest <= 6500);
}

/* The timeslice looked at on the ticks: a turn of 4 ticks (4 ms). */
static void timeslice(void)
{
    pthread_t a, b;
    stop = 0;
    nturns = 0;
    turn = -1;
    pthread_create(&a, NULL, rr, (void *)0);
    pthread_create(&b, NULL, rr, (void *)1);
    struct timespec ts = { 0, 150000000 };
    nanosleep(&ts, NULL);
    stop = 1;
    pthread_join(a, NULL);
    pthread_join(b, NULL);
    /* The first turns may be cut by the threads' start. The emulator
     * delays a tick now and then (on procnto too), which shortens the next
     * turn as much: the turns average 4 ms, and none is missed (8 ms). */
    int good = nturns == 24;
    long sum = 0;
    for (int i = 4; i < nturns; i++) {
        long d = turns[i] - turns[i - 1];
        sum += d;
        if (d < 2000 || d > 6000) {
            printf("schedtest:   turn %d: %ld us\n", i, d);
            good = 0;
        }
    }
    long mean = nturns > 4 ? sum / (nturns - 4) : 0;
    if (mean < 3950 || mean > 4050) {
        printf("schedtest:   turns of %ld us on average\n", mean);
        good = 0;
    }
    check("round-robin threads take turns every 4 ms (3.75 ms on the ticks)", good);
}

static void sporadic_params(void)
{
    struct sched_param sp;
    memset(&sp, 0, sizeof sp);
    sp.sched_priority = 20;
    sp.sched_ss_low_priority = 5;
    sp.sched_ss_max_repl = 4;
    sp.sched_ss_repl_period.tv_nsec = 100000000;
    sp.sched_ss_init_budget.tv_nsec = 20000000;
    struct sched_param bad = sp;
    bad.sched_ss_low_priority = 20;
    errno = 0;
    check("a low priority not below the normal one: EINVAL",
          SchedSet(0, 0, SCHED_SPORADIC, &bad) == -1 && errno == EINVAL);
    bad = sp;
    bad.sched_ss_max_repl = 0;
    errno = 0;
    check("no replenishments: EINVAL", SchedSet(0, 0, SCHED_SPORADIC, &bad) == -1 && errno == EINVAL);
    bad = sp;
    bad.sched_ss_init_budget.tv_nsec = 200000000;
    errno = 0;
    check("a budget over the period: EINVAL", SchedSet(0, 0, SCHED_SPORADIC, &bad) == -1 && errno == EINVAL);
    check("SchedSet(SCHED_SPORADIC)", SchedSet(0, 0, SCHED_SPORADIC, &sp) == 0);
    struct sched_param got;
    memset(&got, 0, sizeof got);
    check("SchedGet(): SCHED_SPORADIC and its parameters",
          SchedGet(0, 0, &got) == SCHED_SPORADIC && got.sched_priority == 20 && got.sched_ss_low_priority == 5 &&
              got.sched_ss_max_repl == 4 && got.sched_ss_repl_period.tv_nsec == 100000000 &&
              got.sched_ss_init_budget.tv_nsec == 20000000);
    sp.sched_priority = 10;
    SchedSet(0, 0, SCHED_RR, &sp);
}

static void *child(void *arg)
{
    struct sched_param sp;
    *(int *)arg = SchedGet(0, 0, &sp) * 1000 + sp.sched_priority;
    return NULL;
}

static void inheritance(void)
{
    struct sched_param sp = { .sched_priority = 15 };
    SchedSet(0, 0, SCHED_FIFO, &sp);
    int got = 0;
    pthread_t c;
    pthread_create(&c, NULL, child, &got);
    pthread_join(c, NULL);
    check("a new thread takes the creator's policy", got == SCHED_FIFO * 1000 + 15);
    pthread_attr_t at;
    pthread_attr_init(&at);
    pthread_attr_setinheritsched(&at, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&at, SCHED_OTHER);
    struct sched_param ap = { .sched_priority = 12 };
    pthread_attr_setschedparam(&at, &ap);
    pthread_create(&c, &at, child, &got);
    pthread_join(c, NULL);
    check("or the attribute's", got == SCHED_OTHER * 1000 + 12);
    sp.sched_priority = 10;
    SchedSet(0, 0, SCHED_RR, &sp);
}

static void *rival(void *arg)
{
    (void)arg;
    pin();
    struct sched_param sp = { .sched_priority = 10 };
    SchedSet(0, 0, SCHED_FIFO, &sp);
    while (!stop) {
    }
    return NULL;
}

/* A sporadic thread (40, low 5; 20 ms every 100 ms) and a priority 10
 * spinner on one CPU: it runs its budget, then nothing until the
 * replenishment. Above the image's services (io-sock at 21), which would
 * otherwise delay a run now and then. */
static void sporadic_runs(void)
{
    pin();
    stop = 0;
    pthread_t r;
    struct sched_param fifo = { .sched_priority = 40 };
    SchedSet(0, 0, SCHED_FIFO, &fifo);
    pthread_create(&r, NULL, rival, NULL);
    struct sched_param sp;
    memset(&sp, 0, sizeof sp);
    sp.sched_priority = 40;
    sp.sched_ss_low_priority = 5;
    sp.sched_ss_max_repl = 4;
    sp.sched_ss_repl_period.tv_nsec = 100000000;
    sp.sched_ss_init_budget.tv_nsec = 20000000;
    SchedSet(0, 0, SCHED_SPORADIC, &sp);
    long t0 = now_us(), last = t0, gaps[4], at[4];
    int n = 0;
    while (n < 3 && now_us() - t0 < 500000) {
        long t = now_us();
        if (t - last > 2000) {
            gaps[n] = t - last;
            at[n++] = last - t0;
        }
        last = t;
    }
    stop = 1;
    fifo.sched_priority = 50;
    SchedSet(0, 0, SCHED_FIFO, &fifo);
    pthread_join(r, NULL);
    int good = n == 3;
    for (int i = 0; i < n; i++) {
        printf("schedtest:   sporadic: ran to %ld us, then waited %ld us\n", at[i], gaps[i]);
        good = good && gaps[i] > 75000 && gaps[i] < 85000;
    }
    check("a sporadic thread runs its budget, then waits for the replenishment", good);
    fifo.sched_priority = 10;
    SchedSet(0, 0, SCHED_RR, &fifo);
}

int main(void)
{
    info();
    timeslice();
    woken_every_tick();
    sporadic_params();
    inheritance();
    sporadic_runs();
    printf("schedtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

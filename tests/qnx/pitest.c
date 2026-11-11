/* Priority inheritance (QNX's default mutex protocol): a mutex's owner
 * runs at the priority of its highest waiter while it waits, through a
 * chain of mutexes too, and back at its own when it unlocks or the waiter
 * gives up; not for a PTHREAD_PRIO_NONE mutex. Waiters get the mutex
 * highest priority first. A raised owner that changes its own priority
 * keeps the raise above it; a sporadic owner keeps it through its budget's
 * end and replenishments. Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("pitest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static int my_priority(void)
{
    struct sched_param sp;
    int policy;
    pthread_getschedparam(pthread_self(), &policy, &sp);
    return sp.sched_curpriority;
}

static pthread_t start(void *(*fn)(void *), void *arg, int prio)
{
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setinheritsched(&a, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&a, SCHED_FIFO);
    struct sched_param sp = {.sched_priority = prio};
    pthread_attr_setschedparam(&a, &sp);
    pthread_t t;
    pthread_create(&t, &a, fn, arg);
    return t;
}

static void *locker(void *arg)
{
    pthread_mutex_lock(arg);
    pthread_mutex_unlock(arg);
    return NULL;
}

static pthread_mutex_t m1 = PTHREAD_MUTEX_INITIALIZER;
static pthread_mutex_t m2 = PTHREAD_MUTEX_INITIALIZER;

/* Holds m2, then waits for m1 (held by main). */
static void *middle(void *arg)
{
    (void)arg;
    pthread_mutex_lock(&m2);
    pthread_mutex_lock(&m1);
    pthread_mutex_unlock(&m1);
    pthread_mutex_unlock(&m2);
    return NULL;
}

static void *timed(void *arg)
{
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    t.tv_nsec += 200 * 1000 * 1000;
    if (t.tv_nsec >= 1000000000) {
        t.tv_sec++;
        t.tv_nsec -= 1000000000;
    }
    return (void *)(long)pthread_mutex_timedlock(arg, &t);
}

static volatile int order[3], ordered;

static void *in_order(void *arg)
{
    pthread_mutex_lock(&m1);
    order[ordered++] = (int)(long)arg;
    pthread_mutex_unlock(&m1);
    return NULL;
}

/* A sporadic owner (20, low 5, a 10 ms budget every 100 ms) holds m1 for
 * 250 ms of spinning; its priorities as they change. */
static volatile int held;
static int seen[16], nseen;

static void *sporadic_owner(void *arg)
{
    (void)arg;
    struct sched_param p;
    memset(&p, 0, sizeof p);
    p.sched_priority = 20;
    p.sched_ss_low_priority = 5;
    p.sched_ss_max_repl = 10;
    p.sched_ss_repl_period.tv_nsec = 100000000;
    p.sched_ss_init_budget.tv_nsec = 10000000;
    pthread_setschedparam(pthread_self(), SCHED_SPORADIC, &p);
    pthread_mutex_lock(&m1);
    held = 1;
    struct timespec now, end;
    clock_gettime(CLOCK_MONOTONIC, &end);
    end.tv_nsec += 250000000;
    if (end.tv_nsec >= 1000000000) {
        end.tv_sec++;
        end.tv_nsec -= 1000000000;
    }
    for (;;) {
        int c = my_priority();
        if ((nseen == 0 || seen[nseen - 1] != c) && nseen < 16) {
            seen[nseen++] = c;
        }
        clock_gettime(CLOCK_MONOTONIC, &now);
        if (now.tv_sec > end.tv_sec || (now.tv_sec == end.tv_sec && now.tv_nsec >= end.tv_nsec)) {
            break;
        }
    }
    pthread_mutex_unlock(&m1);
    if (nseen < 16) {
        seen[nseen++] = my_priority();
    }
    return NULL;
}

int main(void)
{
    struct sched_param sp = {.sched_priority = 10};
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    int base = my_priority();

    pthread_mutex_lock(&m1);
    pthread_t h = start(locker, &m1, 30);
    usleep(100 * 1000);
    int boosted = my_priority();
    pthread_mutex_unlock(&m1);
    int after = my_priority();
    pthread_join(h, NULL);
    check("owner runs at its waiter's priority", base == 10 && boosted == 30 && after == 10);

    pthread_mutexattr_t ma;
    pthread_mutexattr_init(&ma);
    pthread_mutexattr_setprotocol(&ma, PTHREAD_PRIO_NONE);
    pthread_mutex_t none;
    pthread_mutex_init(&none, &ma);
    pthread_mutex_lock(&none);
    h = start(locker, &none, 30);
    usleep(100 * 1000);
    boosted = my_priority();
    pthread_mutex_unlock(&none);
    pthread_join(h, NULL);
    check("none for PTHREAD_PRIO_NONE", boosted == 10);

    pthread_mutex_lock(&m1);
    pthread_t m = start(middle, NULL, 20);
    usleep(100 * 1000);
    h = start(locker, &m2, 30);
    usleep(100 * 1000);
    boosted = my_priority();
    pthread_mutex_unlock(&m1);
    pthread_join(m, NULL);
    pthread_join(h, NULL);
    check("through a chain of mutexes", boosted == 30 && my_priority() == 10);

    pthread_mutex_lock(&m1);
    h = start(timed, &m1, 30);
    usleep(100 * 1000);
    boosted = my_priority();
    void *rc;
    pthread_join(h, &rc);
    int dropped = my_priority();
    pthread_mutex_unlock(&m1);
    check("back when the waiter times out",
          boosted == 30 && (long)rc == ETIMEDOUT && dropped == 10);

    pthread_mutex_lock(&m1);
    pthread_t w[3];
    w[0] = start(in_order, (void *)15L, 15);
    usleep(50 * 1000);
    w[1] = start(in_order, (void *)25L, 25);
    usleep(50 * 1000);
    w[2] = start(in_order, (void *)20L, 20);
    usleep(50 * 1000);
    pthread_mutex_unlock(&m1);
    for (int i = 0; i < 3; i++)
        pthread_join(w[i], NULL);
    check("waiters in priority order", order[0] == 25 && order[1] == 20 && order[2] == 15);

    sp.sched_priority = 20;
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    pthread_mutex_lock(&m1);
    h = start(locker, &m1, 25);
    usleep(10 * 1000);
    pthread_setschedprio(pthread_self(), 10);
    int below = my_priority();
    pthread_setschedprio(pthread_self(), 30);
    int above = my_priority();
    pthread_setschedprio(pthread_self(), 15);
    int back = my_priority();
    pthread_mutex_unlock(&m1);
    int own = my_priority();
    pthread_join(h, NULL);
    check("a raised owner's own priority: the raise stays above it",
          below == 25 && above == 30 && back == 25 && own == 15);

    /* All on one CPU, the main thread above them: the owner runs out of
     * budget (5), then a waiter raises it (25) until it unlocks. */
    unsigned mask = 1;
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)mask);
    sp.sched_priority = 40;
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    pthread_create(&h, NULL, sporadic_owner, NULL);
    while (!held) {
        usleep(1000);
    }
    usleep(30 * 1000);
    pthread_t lw = start(locker, &m1, 25);
    pthread_join(h, NULL);
    pthread_join(lw, NULL);
    for (int i = 0; i < nseen; i++) {
        printf("pitest:   sporadic owner at %d\n", seen[i]);
    }
    check("a sporadic owner keeps the raise through its budget's end and replenishments",
          nseen == 4 && seen[0] == 20 && seen[1] == 5 && seen[2] == 25 && seen[3] == 5);

    printf("pitest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

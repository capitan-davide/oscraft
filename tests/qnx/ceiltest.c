/* Priority-ceiling mutexes held together (review item SCHED-9), as
 * procnto (measured): a thread runs at the highest ceiling among the
 * mutexes it still holds, whatever order it unlocks them in, and at its
 * own priority once it holds none. The kernel used to remember, per
 * mutex, the priority the owner ran at when it took it and restore that
 * on unlock, so unlocking out of order left the thread at the wrong
 * priority (10 under a ceiling of 30, or 20 after unlocking both). Also: a
 * condition variable wait on a ceiling mutex, a priority change while a
 * ceiling is held, a mutex whose ceiling is below the ceiling the thread
 * runs at, and one below the thread's own priority (EINVAL). Prints one
 * "ok"/"FAIL" line per check and "ceiltest: passed". */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <time.h>

static int fails;

static void check(const char *what, int got, int want)
{
    printf("ceiltest: %s %s (%d, want %d)\n", got == want ? "ok" : "FAIL", what, got, want);
    fflush(stdout);
    fails += got != want;
}

/* the calling thread's current (effective) priority */
static int cur(void)
{
    struct sched_param p;
    int pol;
    pthread_getschedparam(pthread_self(), &pol, &p);
    return p.sched_curpriority;
}

static void setprio(int prio)
{
    struct sched_param sp = { .sched_priority = prio };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
}

static void mk(pthread_mutex_t *m, int ceil)
{
    pthread_mutexattr_t a;
    pthread_mutexattr_init(&a);
    pthread_mutexattr_setprotocol(&a, PTHREAD_PRIO_PROTECT);
    pthread_mutexattr_setprioceiling(&a, ceil);
    check("mutex created", pthread_mutex_init(m, &a), 0);
}

int main(void)
{
    pthread_mutex_t a, b;
    setprio(10);
    mk(&a, 20);
    mk(&b, 30);
    check("own priority", cur(), 10);

    /* taken A then B, unlocked A first */
    check("lock A", pthread_mutex_lock(&a), 0);
    check("under A (ceiling 20)", cur(), 20);
    check("lock B", pthread_mutex_lock(&b), 0);
    check("under A and B (ceiling 30)", cur(), 30);
    check("unlock A", pthread_mutex_unlock(&a), 0);
    check("A unlocked first, B still held", cur(), 30);
    check("unlock B", pthread_mutex_unlock(&b), 0);
    check("both unlocked", cur(), 10);

    /* taken B then A (below the ceiling it runs at), unlocked B first */
    check("lock B", pthread_mutex_lock(&b), 0);
    check("lock A at ceiling 30", pthread_mutex_lock(&a), 0);
    check("under B and A", cur(), 30);
    check("unlock B", pthread_mutex_unlock(&b), 0);
    check("B unlocked first, A still held", cur(), 20);
    check("unlock A", pthread_mutex_unlock(&a), 0);
    check("both unlocked again", cur(), 10);

    /* a condition variable wait on B, timed out */
    pthread_cond_t c = PTHREAD_COND_INITIALIZER;
    pthread_mutex_lock(&b);
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_nsec += 20000000;
    if (ts.tv_nsec >= 1000000000) {
        ts.tv_sec++;
        ts.tv_nsec -= 1000000000;
    }
    check("cond_timedwait on B", pthread_cond_timedwait(&c, &b, &ts), ETIMEDOUT);
    check("after the wait, B held again", cur(), 30);
    check("unlock B", pthread_mutex_unlock(&b), 0);
    check("after the wait, unlocked", cur(), 10);

    /* the thread's own priority changed while it holds a ceiling */
    pthread_mutex_lock(&b);
    setprio(15);
    check("own priority 15 under B", cur(), 30);
    check("unlock B", pthread_mutex_unlock(&b), 0);
    check("own priority 15, unlocked", cur(), 15);

    /* above the ceiling */
    setprio(25);
    int r = pthread_mutex_lock(&a);
    check("at 25, lock A (ceiling 20)", r, EINVAL);
    if (r == 0)
        pthread_mutex_unlock(&a);
    check("at 25 after the failed lock", cur(), 25);

    printf("ceiltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Barriers (pthread_barrier_wait(): every thread through each round, one
 * serial thread per round), priority-ceiling mutexes (the owner runs at
 * the ceiling, pthread_mutex_setprioceiling()), sigqueue() values, and
 * ClockAdjust(). Prints one "ok"/"FAIL" line per case. */
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("synctest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

#define THREADS 4
#define ROUNDS 50

static pthread_barrier_t barrier;
static volatile int arrived[ROUNDS];
static volatile int serial[ROUNDS];
static volatile int early;

static void *worker(void *arg)
{
    (void)arg;
    for (int r = 0; r < ROUNDS; r++) {
        __atomic_add_fetch(&arrived[r], 1, __ATOMIC_SEQ_CST);
        int rc = pthread_barrier_wait(&barrier);
        if (arrived[r] != THREADS)
            early = 1;
        if (rc == PTHREAD_BARRIER_SERIAL_THREAD)
            __atomic_add_fetch(&serial[r], 1, __ATOMIC_SEQ_CST);
        else if (rc != 0)
            early = 2;
    }
    return NULL;
}

static int my_priority(void)
{
    struct sched_param sp;
    int policy;
    pthread_getschedparam(pthread_self(), &policy, &sp);
    return sp.sched_curpriority;
}

int main(void)
{
    pthread_barrier_init(&barrier, NULL, THREADS);
    pthread_t t[THREADS];
    for (int i = 0; i < THREADS; i++)
        pthread_create(&t[i], NULL, worker, NULL);
    for (int i = 0; i < THREADS; i++)
        pthread_join(t[i], NULL);
    int one_serial = 1;
    for (int r = 0; r < ROUNDS; r++)
        one_serial &= serial[r] == 1;
    check("barrier: nobody leaves early", early == 0);
    check("barrier: one serial thread per round", one_serial);
    check("barrier destroyed", pthread_barrier_destroy(&barrier) == 0);

    pthread_mutexattr_t ma;
    pthread_mutexattr_init(&ma);
    pthread_mutexattr_setprotocol(&ma, PTHREAD_PRIO_PROTECT);
    pthread_mutexattr_setprioceiling(&ma, 40);
    pthread_mutex_t pm;
    int rc = pthread_mutex_init(&pm, &ma);
    check("priority-ceiling mutex created", rc == 0);
    int before = my_priority();
    pthread_mutex_lock(&pm);
    int inside = my_priority();
    pthread_mutex_unlock(&pm);
    int after = my_priority();
    check("owner runs at the ceiling", inside == 40 && before == after && before < 40);
    int old = 0, now = 0;
    rc = pthread_mutex_setprioceiling(&pm, 30, &old);
    pthread_mutex_getprioceiling(&pm, &now);
    check("ceiling changed", rc == 0 && old == 40 && now == 30);
    pthread_mutex_lock(&pm);
    inside = my_priority();
    pthread_mutex_unlock(&pm);
    check("owner runs at the new ceiling", inside == 30);
    pthread_mutex_destroy(&pm);

    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigprocmask(SIG_BLOCK, &set, NULL);
    union sigval v = {.sival_int = 12345};
    rc = sigqueue(getpid(), SIGUSR1, v);
    siginfo_t info;
    int got = sigwaitinfo(&set, &info);
    check("sigqueue() value", rc == 0 && got == SIGUSR1 && info.si_value.sival_int == 12345 &&
                                  info.si_code == SI_QUEUE);

    /* Speed the clock up by 10 us per tick for 100 ticks, then stop. */
    struct _clockadjust adj = {.tick_count = 100, .tick_nsec_inc = 10000}, prev;
    rc = ClockAdjust(CLOCK_REALTIME, &adj, &prev);
    struct _clockadjust none = {0, 0}, now_adj;
    int rc2 = ClockAdjust(CLOCK_REALTIME, &none, &now_adj);
    check("ClockAdjust()", rc == 0 && rc2 == 0 && now_adj.tick_nsec_inc == 10000 &&
                               now_adj.tick_count > 0 && now_adj.tick_count <= 100);

    printf("synctest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

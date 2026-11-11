/* Threads and synchronization objects as libc and procnto give them:
 * mutex types (error-checking, recursive, normal) and their errors,
 * timed locks, condition variable clocks, read-write locks, barriers,
 * spin locks, thread attributes, keys, semaphores; priority ceilings
 * changed by the holder and by another thread (which waits for the
 * mutex), pthread_mutex_wakeup_np(). Every call's result is
 * procnto's (measured in the reference image): pthread functions return
 * the error, so each check compares the return value. A
 * pthread_cond_timedwait() whose timeout fires while another thread
 * holds the mutex is still ETIMEDOUT (review item SCHED-2). Prints one
 * "ok"/"FAIL" line per call and "sync2test: passed". */
#include <stdio.h>
#include <errno.h>
#include <string.h>
#include <pthread.h>
#include <semaphore.h>
#include <signal.h>
#include <time.h>
#include <unistd.h>
#include <limits.h>
#include <sched.h>
#include <sys/neutrino.h>
static int fails;

/* `expr` gives `val`. */
#define V(expr, val)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == (long)(val);                                                                   \
        printf("sync2test: %s %s (%ld)\n", ok_ ? "ok" : "FAIL", #expr, r_);                             \
        fails += !ok_;                                                                                 \
    } while (0)

static pthread_mutex_t gm = PTHREAD_MUTEX_INITIALIZER;
static void *locker(void *a) { pthread_mutex_lock((pthread_mutex_t *)a); return 0; }
static void *unlocker(void *a) { return (void *)(long)pthread_mutex_unlock((pthread_mutex_t *)a); }
static int curprio(void)
{
    struct sched_param sp;
    SchedGet(0, 0, &sp);
    return sp.sched_curpriority;
}

static long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static pthread_mutex_t cm, wm;
static volatile int held, wres = -99, wtid;
static volatile long t_rel, t_set;

static void *ceil_holder(void *a)
{
    (void)a;
    pthread_mutex_lock(&cm);
    held = 1;
    usleep(100000);
    t_rel = now_ms();
    pthread_mutex_unlock(&cm);
    return NULL;
}

static void *ceil_setter(void *a)
{
    int old;
    *(int *)a = pthread_mutex_setprioceiling(&cm, 40, &old) * 100 + old;
    t_set = now_ms();
    return NULL;
}

static void *wake_holder(void *a)
{
    (void)a;
    pthread_mutex_lock(&wm);
    held = 2;
    usleep(300000);
    pthread_mutex_unlock(&wm);
    return NULL;
}

static void *wake_waiter(void *a)
{
    (void)a;
    wtid = pthread_self();
    wres = pthread_mutex_lock(&wm);
    if (wres == 0) {
        pthread_mutex_unlock(&wm);
    }
    return NULL;
}

/* Priority ceilings changed while held, and by a thread that doesn't hold
 * the mutex; waking a waiter of a PTHREAD_WAKEUP_ENABLE mutex. */
static void ceilings_and_wakeup(void)
{
    struct sched_param sp = { .sched_priority = 10 };
    pthread_setschedparam(pthread_self(), SCHED_RR, &sp);
    pthread_mutexattr_t ma;
    pthread_mutexattr_init(&ma);
    pthread_mutexattr_setprotocol(&ma, PTHREAD_PRIO_PROTECT);
    pthread_mutexattr_setprioceiling(&ma, 30);
    pthread_mutex_init(&cm, &ma);
    int old;
    pthread_mutex_lock(&cm);
    V(curprio(), 30);
    pthread_mutex_setprioceiling(&cm, 20, &old);
    V(curprio(), 20);
    pthread_mutex_setprioceiling(&cm, 5, &old);
    V(curprio(), 10);
    pthread_mutex_setprioceiling(&cm, 40, &old);
    V(curprio(), 40);
    pthread_mutex_unlock(&cm);
    V(curprio(), 10);
    pthread_mutex_setprioceiling(&cm, 30, &old);
    pthread_t h, s;
    int res = -1;
    pthread_create(&h, NULL, ceil_holder, NULL);
    while (!held) {
        usleep(1000);
    }
    pthread_create(&s, NULL, ceil_setter, &res);
    pthread_join(h, NULL);
    pthread_join(s, NULL);
    V(res, 30);
    V(t_set >= t_rel, 1);

    pthread_mutexattr_t wa;
    pthread_mutexattr_init(&wa);
    V(pthread_mutexattr_setwakeup_np(&wa, PTHREAD_WAKEUP_ENABLE), 0);
    pthread_mutex_init(&wm, &wa);
    pthread_t a, b;
    pthread_create(&a, NULL, wake_holder, NULL);
    while (held != 2) {
        usleep(1000);
    }
    pthread_create(&b, NULL, wake_waiter, NULL);
    usleep(50000);
    V(pthread_mutex_wakeup_np(&wm, 0, wtid), 0);
    pthread_join(b, NULL);
    V(wres, EINTR);
    pthread_join(a, NULL);
    pthread_mutex_t plain = PTHREAD_MUTEX_INITIALIZER;
    V(pthread_mutex_wakeup_np(&plain, 0, 0), EINVAL);
}

static pthread_mutex_t hm = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t hc = PTHREAD_COND_INITIALIZER;

/* Takes the mutex 40 ms in, holds it 80 ms, never signals. */
static void *holder(void *arg)
{
    (void)arg;
    usleep(40000);
    pthread_mutex_lock(&hm);
    usleep(80000);
    pthread_mutex_unlock(&hm);
    return NULL;
}

/* The timeout (50 ms) fires while the holder has the mutex: the wait
 * ends ETIMEDOUT once the mutex is back, not 0. */
static void timeout_under_holder(void)
{
    for (int round = 0; round < 3; round++) {
        pthread_t h;
        struct timespec until;
        clock_gettime(CLOCK_REALTIME, &until);
        until.tv_nsec += 50000000;
        if (until.tv_nsec >= 1000000000) {
            until.tv_sec++;
            until.tv_nsec -= 1000000000;
        }
        pthread_mutex_lock(&hm);
        pthread_create(&h, NULL, holder, NULL);
        V(pthread_cond_timedwait(&hc, &hm, &until), ETIMEDOUT);
        pthread_mutex_unlock(&hm);
        pthread_join(h, NULL);
    }
}

int main(void)
{
    pthread_mutexattr_t ma; pthread_mutex_t m, r, e;
    pthread_mutexattr_init(&ma);
    V(pthread_mutexattr_settype(&ma, 99), 22);
    V(pthread_mutexattr_settype(&ma, PTHREAD_MUTEX_ERRORCHECK), 0);
    pthread_mutex_init(&e, &ma);
    V(pthread_mutex_lock(&e), 0);
    V(pthread_mutex_lock(&e), 45);
    V(pthread_mutex_unlock(&e), 0);
    V(pthread_mutex_unlock(&e), 1);
    V(pthread_mutexattr_settype(&ma, PTHREAD_MUTEX_RECURSIVE), 0);
    pthread_mutex_init(&r, &ma);
    V(pthread_mutex_lock(&r), 0);
    V(pthread_mutex_lock(&r), 0);
    V(pthread_mutex_trylock(&r), 0);
    V(pthread_mutex_unlock(&r), 0);
    V(pthread_mutex_unlock(&r), 0);
    V(pthread_mutex_unlock(&r), 0);
    V(pthread_mutex_unlock(&r), 1);
    pthread_mutexattr_settype(&ma, PTHREAD_MUTEX_NORMAL); pthread_mutex_init(&m, &ma);
    V(pthread_mutex_unlock(&m), 1);
    V(pthread_mutex_trylock(&m), 0);
    V(pthread_mutex_trylock(&m), 16);
    pthread_t t; void *rv; pthread_create(&t, 0, unlocker, &m); pthread_join(t, &rv);
    V((long)rv, 1);
    V(pthread_mutex_unlock(&m), 0);
    V(pthread_mutex_destroy(&m), 0);
    pthread_mutex_lock(&gm);
    V(pthread_mutex_destroy(&gm), 0);
    pthread_mutex_unlock(&gm);
    V(pthread_mutexattr_setprotocol(&ma, 99), 22);
    V(pthread_mutexattr_setprioceiling(&ma, 300), 22);
    V(pthread_mutexattr_setpshared(&ma, 99), 22);
    struct timespec ts; clock_gettime(CLOCK_REALTIME, &ts); ts.tv_nsec += 10000000; if (ts.tv_nsec >= 1000000000) { ts.tv_sec++; ts.tv_nsec -= 1000000000; }
    pthread_mutex_lock(&gm); pthread_create(&t, 0, locker, &gm);
    usleep(10000);
    pthread_mutex_t tm = PTHREAD_MUTEX_INITIALIZER; pthread_mutex_lock(&tm);
    V(pthread_mutex_timedlock(&tm, &ts), 45);
    struct timespec bad = {0, 2000000000L};
    V(pthread_mutex_timedlock(&tm, &bad), 45);
    pthread_mutex_unlock(&tm); pthread_mutex_unlock(&gm); pthread_join(t, 0); pthread_mutex_unlock(&gm);
    pthread_cond_t c; pthread_condattr_t ca; pthread_condattr_init(&ca);
    V(pthread_condattr_setclock(&ca, CLOCK_MONOTONIC), 0);
    V(pthread_condattr_setclock(&ca, 99), 22);
    V(pthread_condattr_setclock(&ca, CLOCK_PROCESS_CPUTIME_ID), 22);
    pthread_condattr_setclock(&ca, CLOCK_MONOTONIC); pthread_cond_init(&c, &ca);
    pthread_mutex_lock(&tm); struct timespec mt; clock_gettime(CLOCK_MONOTONIC, &mt); mt.tv_nsec += 5000000; if (mt.tv_nsec >= 1000000000) { mt.tv_sec++; mt.tv_nsec -= 1000000000; }
    V(pthread_cond_timedwait(&c, &tm, &mt), 260);
    V(pthread_cond_timedwait(&c, &tm, &bad), 22);
    pthread_mutex_unlock(&tm);
    V(pthread_cond_wait(&c, &tm), 1);
    V(pthread_cond_destroy(&c), 0);
    pthread_rwlock_t rw = PTHREAD_RWLOCK_INITIALIZER;
    V(pthread_rwlock_rdlock(&rw), 0);
    V(pthread_rwlock_trywrlock(&rw), 16);
    V(pthread_rwlock_tryrdlock(&rw), 0);
    V(pthread_rwlock_unlock(&rw), 0);
    V(pthread_rwlock_unlock(&rw), 0);
    V(pthread_rwlock_unlock(&rw), 22);
    V(pthread_rwlock_wrlock(&rw), 0);
    V(pthread_rwlock_wrlock(&rw), 45);
    V(pthread_rwlock_rdlock(&rw), 45);
    V(pthread_rwlock_unlock(&rw), 0);
    pthread_barrier_t b;
    V(pthread_barrier_init(&b, 0, 0), 22);
    V(pthread_barrier_init(&b, 0, 1), 0);
    V(pthread_barrier_wait(&b), -1);
    V(pthread_barrier_destroy(&b), 0);
    pthread_spinlock_t sp;
    V(pthread_spin_init(&sp, 0), 0);
    V(pthread_spin_lock(&sp), 0);
    V(pthread_spin_trylock(&sp), 16);
    V(pthread_spin_unlock(&sp), 0);
    V(pthread_spin_destroy(&sp), 0);
    pthread_attr_t ta; pthread_attr_init(&ta);
    V(pthread_attr_setstacksize(&ta, 100), 22);
    V(pthread_attr_setstacksize(&ta, PTHREAD_STACK_MIN), 0);
    V(pthread_attr_setdetachstate(&ta, 99), 22);
    V(pthread_attr_setinheritsched(&ta, 99), 22);
    V(pthread_attr_setschedpolicy(&ta, 99), 22);
    V(pthread_attr_setscope(&ta, PTHREAD_SCOPE_PROCESS), 48);
    V(pthread_attr_setguardsize(&ta, 12345), 0);
    size_t ss; pthread_attr_init(&ta); pthread_attr_getstacksize(&ta, &ss);
    V(ss, 262144);
    pthread_attr_getguardsize(&ta, &ss);
    V(ss, 4096);
    struct sched_param sp2; sp2.sched_priority = 300; pthread_attr_setinheritsched(&ta, PTHREAD_EXPLICIT_SCHED); pthread_attr_setschedparam(&ta, &sp2);
    V(pthread_create(&t, &ta, locker, &gm), 22);
    V(pthread_join(pthread_self(), 0), 45);
    V(pthread_detach(9999), 3);
    V(pthread_kill(9999, 0), 3);
    V(pthread_kill(pthread_self(), 99), 22);
    V(pthread_setschedprio(pthread_self(), 300), 22);
    V(pthread_setschedprio(pthread_self(), 0), 22);
    int pol; struct sched_param p3; pthread_getschedparam(pthread_self(), &pol, &p3);
    V(pol, SCHED_RR);
    V(p3.sched_priority, 10);
    pthread_key_t k;
    V(pthread_key_create(&k, 0), 0);
    V(pthread_key_delete(k), 0);
    V(pthread_key_delete(k), 22);
    V(pthread_setspecific(9999, 0), 22);
    sem_t s;
    V(sem_init(&s, 0, SEM_VALUE_MAX), 0);
    V(sem_post(&s), -1);
    V(sem_init(&s, 0, (unsigned)SEM_VALUE_MAX + 1), -1);
    V(pthread_setcancelstate(99, 0), 22);
    V(pthread_setcanceltype(99, 0), 22);
    ceilings_and_wakeup();
    timeout_under_holder();
    printf("sync2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

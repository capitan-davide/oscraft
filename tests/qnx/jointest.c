/* Threads: pthread_join() of a running and of a finished thread, their
 * exit values, pthread_detach(), thread names, semaphores, read-write
 * locks, scheduling policy limits; a second pthread_join() on a thread
 * another thread is joining is EBUSY (review item SCHED-3). Prints one
 * "ok"/"FAIL" line per case. */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("jointest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void *slow(void *arg)
{
    usleep(100 * 1000);
    return (char *)arg + 1;
}

static void *quick(void *arg)
{
    pthread_exit((char *)arg + 2);
}

static void *forever(void *arg)
{
    (void)arg;
    for (;;) {
        sleep(10);
    }
}

static sem_t sem;

static void *poster(void *arg)
{
    (void)arg;
    usleep(50 * 1000);
    sem_post(&sem);
    return NULL;
}

/* An absolute CLOCK_REALTIME time `ms` from now. */
static struct timespec in_ms(long ms)
{
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    t.tv_nsec += ms * 1000 * 1000;
    if (t.tv_nsec >= 1000000000) {
        t.tv_sec++;
        t.tv_nsec -= 1000000000;
    }
    return t;
}

static void semaphores(void)
{
    int v = -1;
    pthread_t t;

    check("sem_init", sem_init(&sem, 0, 1) == 0);
    check("sem_trywait takes a unit", sem_trywait(&sem) == 0);
    check("sem_trywait on zero fails", sem_trywait(&sem) == -1 && errno == EAGAIN);
    pthread_create(&t, NULL, poster, NULL);
    check("sem_wait woken by sem_post", sem_wait(&sem) == 0);
    pthread_join(t, NULL);
    sem_post(&sem);
    sem_post(&sem);
    check("sem_getvalue", sem_getvalue(&sem, &v) == 0 && v == 2);
    sem_wait(&sem);
    sem_wait(&sem);
    struct timespec until = in_ms(100);
    check("sem_timedwait times out", sem_timedwait(&sem, &until) == -1 && errno == ETIMEDOUT);
    check("sem_destroy", sem_destroy(&sem) == 0);
    check("sem_post on a destroyed semaphore fails", sem_post(&sem) == -1 && errno == EINVAL);
}

static pthread_rwlock_t rw = PTHREAD_RWLOCK_INITIALIZER;
static volatile int writer_done;

static void *writer(void *arg)
{
    (void)arg;
    pthread_rwlock_wrlock(&rw);
    usleep(100 * 1000);
    writer_done = 1;
    pthread_rwlock_unlock(&rw);
    return NULL;
}

static void rwlocks(void)
{
    pthread_t t;

    pthread_create(&t, NULL, writer, NULL);
    usleep(30 * 1000);
    check("tryrdlock while a writer holds fails", pthread_rwlock_tryrdlock(&rw) == EBUSY);
    check("rdlock waits for the writer", pthread_rwlock_rdlock(&rw) == 0 && writer_done);
    check("a second reader shares", pthread_rwlock_tryrdlock(&rw) == 0);
    check("trywrlock while readers hold fails", pthread_rwlock_trywrlock(&rw) == EBUSY);
    pthread_rwlock_unlock(&rw);
    writer_done = 0;
    pthread_create(&t, NULL, writer, NULL);
    usleep(30 * 1000);
    /* The writer waits for our read lock; it gets it as we let go. */
    pthread_rwlock_unlock(&rw);
    pthread_join(t, NULL);
    check("a waiting writer gets the lock as the readers leave", writer_done);
    check("wrlock", pthread_rwlock_wrlock(&rw) == 0);
    pthread_rwlock_unlock(&rw);
    check("rwlock destroy", pthread_rwlock_destroy(&rw) == 0);
}

/* FP/SIMD state stays the thread's own across switches: each thread keeps
 * its value in a vector register while it yields to the others. */
static void *fp_keeper(void *arg)
{
    double v = (double)(long)arg + 0.5, got = 0;
    for (int i = 0; i < 2000; i++) {
        __asm__ volatile("fmov d8, %d0" ::"w"(v) : "d8");
        sched_yield();
        __asm__ volatile("fmov %d0, d8" : "=w"(got));
        if (got != v)
            return (void *)1;
    }
    return NULL;
}

/* libc's kernel call stub (no header declares it): CPUPAGE_CPU is the
 * CPU the caller runs on. */
extern intptr_t __SysCpupageGet(int index);

/* Pinned to CPU (long)arg: runs nowhere else while it yields. */
static void *pinned(void *arg)
{
    long cpu = (long)arg;
    if (ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(1UL << cpu)) == -1)
        return (void *)-1L;
    for (int i = 0; i < 200; i++) {
        if (__SysCpupageGet(CPUPAGE_CPU) != cpu)
            return (void *)-1L;
        sched_yield();
    }
    return NULL;
}

static sem_t join_go;
static pthread_t join_target;
static int first_join = -1;

static void *join_waiter(void *arg)
{
    (void)arg;
    sem_wait(&join_go);
    return NULL;
}

static void *first_joiner(void *arg)
{
    (void)arg;
    first_join = pthread_join(join_target, NULL);
    return NULL;
}

/* One thread joins a blocked thread; a second join of it is EBUSY. */
static void double_join(void)
{
    pthread_t j;
    sem_init(&join_go, 0, 0);
    pthread_create(&join_target, NULL, join_waiter, NULL);
    pthread_create(&j, NULL, first_joiner, NULL);
    usleep(50 * 1000);
    check("a second join while another thread joins: EBUSY", pthread_join(join_target, NULL) == EBUSY);
    sem_post(&join_go);
    pthread_join(j, NULL);
    check("the first joiner gets it", first_join == 0);
    check("joining it again: ESRCH", pthread_join(join_target, NULL) == ESRCH);
}

int main(void)
{
    char base[4];
    pthread_t t;
    void *ret = NULL;

    pthread_create(&t, NULL, slow, base);
    check("join a running thread", pthread_join(t, &ret) == 0 && ret == base + 1);

    pthread_create(&t, NULL, quick, base);
    usleep(50 * 1000);
    ret = NULL;
    check("join a finished thread", pthread_join(t, &ret) == 0 && ret == base + 2);
    check("join twice fails", pthread_join(t, &ret) != 0);

    pthread_create(&t, NULL, slow, base);
    check("detach", pthread_detach(t) == 0);
    check("join a detached thread fails", pthread_join(t, &ret) == EINVAL);

    check("join self fails", pthread_join(pthread_self(), &ret) == EDEADLK);

    pthread_create(&t, NULL, forever, NULL);
    struct timespec until = in_ms(100);
    check("timed join times out", pthread_timedjoin(t, &ret, &until) == ETIMEDOUT);

    char name[32] = {0};
    check("set name", pthread_setname_np(t, "worker") == 0);
    check("get name", pthread_getname_np(t, name, sizeof name) == 0 && strcmp(name, "worker") == 0);

    semaphores();
    rwlocks();
    /* As procnto for root: 254 and 255 are its clock and IPI threads'. */
    check("priority range", sched_get_priority_min(SCHED_RR) == 1 && sched_get_priority_max(SCHED_RR) == 253);
    check("unknown policy fails", sched_get_priority_max(99) == -1 && errno == EINVAL);

    pthread_t keepers[4];
    int fp_ok = 1;
    for (long i = 0; i < 4; i++)
        pthread_create(&keepers[i], NULL, fp_keeper, (void *)i);
    for (int i = 0; i < 4; i++) {
        void *r = (void *)1;
        pthread_join(keepers[i], &r);
        fp_ok &= r == NULL;
    }
    check("FP registers are each thread's own", fp_ok);

    /* Runmasks: a thread on each CPU, and none on a CPU that's offline. */
    int pinned_ok = 1;
    for (long cpu = 0; cpu < _syspage_ptr->num_cpu; cpu++) {
        void *r = (void *)1;
        pthread_create(&t, NULL, pinned, (void *)cpu);
        pthread_join(t, &r);
        pinned_ok &= r == NULL;
    }
    check("runmask pins a thread to a CPU", pinned_ok);
    check("a runmask of no online CPU", ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(1UL << 40)) == -1 && errno == EINVAL);

    /* Thread IDs: the lowest free one. A joined thread's is free; an
     * ended thread's not until it is joined; a detached one's as it ends. */
    pthread_t ids[4];
    for (int i = 0; i < 3; i++) {
        pthread_create(&ids[i], NULL, quick, NULL);
    }
    usleep(20000);
    pthread_join(ids[0], NULL);
    pthread_t again;
    pthread_create(&again, NULL, quick, NULL);
    printf("jointest:   ids %d %d %d, after joining the first: %d\n", ids[0], ids[1], ids[2], again);
    check("a joined thread's ID is the next one's", again == ids[0]);
    pthread_create(&ids[3], NULL, quick, NULL);
    printf("jointest:   with %d and %d ended, not joined: %d\n", ids[1], ids[2], ids[3]);
    check("an ended thread's ID is in use until it is joined", ids[3] > ids[2]);
    pthread_join(ids[1], NULL);
    pthread_join(ids[2], NULL);
    pthread_join(again, NULL);
    pthread_join(ids[3], NULL);
    pthread_attr_t da;
    pthread_attr_init(&da);
    pthread_attr_setdetachstate(&da, PTHREAD_CREATE_DETACHED);
    pthread_t d, after;
    pthread_create(&d, &da, quick, NULL);
    usleep(20000);
    pthread_create(&after, NULL, quick, NULL);
    printf("jointest:   detached %d ended, then %d\n", d, after);
    check("a detached thread's ID is free once it ends", after == d);
    pthread_join(after, NULL);

    double_join();
    printf("jointest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

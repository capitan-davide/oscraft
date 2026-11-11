/* Robust mutexes in detail, as procnto (measured):
 * - a process-shared mutex has _NTO_SYNC_SHARED | _NTO_SYNC_KFORCE;
 * - taken from a dead owner (EOWNERDEAD), a mutex keeps __owner
 *   _NTO_SYNC_OWNERDEAD until made consistent; unlocked without that it
 *   becomes _NTO_SYNC_DESTROYED, and locks fail with ENOTRECOVERABLE;
 * - when the owner process ends holding a shared one, its __owner becomes
 *   _NTO_SYNC_OWNERDEAD at once, and a waiter in another process takes it
 *   with EOWNERDEAD;
 * - a shared mutex whose owner thread ended in a process that lives on:
 *   lock EINVAL, trylock EBUSY;
 * - trylock doesn't notice a dead owner thread: EBUSY.
 * Prints one "ok"/"FAIL" line per check and "robust2test: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define OWNERDEAD 0xffffff00u
#define DESTROYED 0xfffffffeu
#define WAITING 0x80000000u

static int fails;
static pthread_mutex_t *m;

static void check(const char *what, int ok)
{
    printf("robust2test: %s %s (count %#x owner %#x)\n", ok ? "ok" : "FAIL", what, m->__u.__count, m->__owner);
    fails += !ok;
}

static pthread_mutex_t pool[4];
static int npool;

static pthread_mutex_t *fresh(int pshared, int robust)
{
    pthread_mutex_t *x = pshared ? mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, NOFD, 0) : &pool[npool++];
    pthread_mutexattr_t at;
    pthread_mutexattr_init(&at);
    if (robust)
        pthread_mutexattr_setrobust(&at, PTHREAD_MUTEX_ROBUST);
    if (pshared)
        pthread_mutexattr_setpshared(&at, PTHREAD_PROCESS_SHARED);
    pthread_mutex_init(x, &at);
    return x;
}

static void *locker(void *a)
{
    pthread_mutex_lock(m);
    return a;
}

/* A thread locks `m` and ends. */
static void thread_dies(void)
{
    pthread_t t;
    pthread_create(&t, NULL, locker, NULL);
    pthread_join(t, NULL);
}

/* A child process locks `m` and ends (after `ms`). */
static pid_t process_dies(int ms)
{
    pid_t p = fork();
    if (p == 0) {
        pthread_mutex_lock(m);
        usleep(ms * 1000);
        _exit(0);
    }
    return p;
}

static int lock_secs(int secs)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_sec += secs;
    return pthread_mutex_timedlock(m, &ts);
}

int main(void)
{
    unsigned me = (getpid() & 0x7fff) << 16 | 1;

    m = fresh(1, 0);
    check("a shared mutex: _NTO_SYNC_SHARED | _NTO_SYNC_KFORCE", (m->__u.__count & 0x44000000) == 0x44000000);
    m = fresh(0, 1);
    check("a private robust one: no _NTO_SYNC_KFORCE", (m->__u.__count & 0x04000000) == 0);

    /* Private, owner thread ended, unlocked inconsistent. */
    thread_dies();
    check("trylock with a dead owner thread: EBUSY", pthread_mutex_trylock(m) == EBUSY);
    check("lock: EOWNERDEAD, __owner _NTO_SYNC_OWNERDEAD", lock_secs(1) == EOWNERDEAD && m->__owner == OWNERDEAD);
    check("unlock inconsistent: __owner _NTO_SYNC_DESTROYED", pthread_mutex_unlock(m) == 0 && m->__owner == DESTROYED);
    check("lock: ENOTRECOVERABLE", lock_secs(1) == ENOTRECOVERABLE);
    check("trylock: ENOTRECOVERABLE", pthread_mutex_trylock(m) == ENOTRECOVERABLE);
    check("consistent: EINVAL", pthread_mutex_consistent(m) == EINVAL);

    /* Private, made consistent. */
    m = fresh(0, 1);
    thread_dies();
    check("lock: EOWNERDEAD", lock_secs(1) == EOWNERDEAD);
    check("consistent: __owner ours", pthread_mutex_consistent(m) == 0 && (m->__owner & ~WAITING) == me);
    check("unlock, lock again", pthread_mutex_unlock(m) == 0 && lock_secs(1) == 0 && pthread_mutex_unlock(m) == 0);

    /* Shared, owner process ended. */
    m = fresh(1, 1);
    waitpid(process_dies(0), NULL, 0);
    check("the owner process ended: __owner _NTO_SYNC_OWNERDEAD at once", m->__owner == OWNERDEAD);
    check("lock: EOWNERDEAD", lock_secs(1) == EOWNERDEAD && m->__owner == OWNERDEAD);
    check("consistent: __owner ours", pthread_mutex_consistent(m) == 0 && (m->__owner & ~WAITING) == me);
    check("unlock", pthread_mutex_unlock(m) == 0 && m->__owner == 0);

    m = fresh(1, 1);
    waitpid(process_dies(0), NULL, 0);
    check("lock: EOWNERDEAD", lock_secs(1) == EOWNERDEAD);
    check("unlock inconsistent: _NTO_SYNC_DESTROYED", pthread_mutex_unlock(m) == 0 && m->__owner == DESTROYED);
    check("lock: ENOTRECOVERABLE", lock_secs(1) == ENOTRECOVERABLE);

    /* Shared, a waiter when the owner process ends. */
    m = fresh(1, 1);
    pid_t p = process_dies(300);
    usleep(100000);
    check("a waiter takes it as the owner process ends: EOWNERDEAD", lock_secs(2) == EOWNERDEAD);
    waitpid(p, NULL, 0);
    pthread_mutex_consistent(m);
    pthread_mutex_unlock(m);

    /* Shared, owner thread ended, its process lives on. */
    m = fresh(1, 1);
    thread_dies();
    check("shared, owner thread ended: trylock EBUSY", pthread_mutex_trylock(m) == EBUSY);
    check("lock EINVAL", lock_secs(1) == EINVAL);
    check("consistent EINVAL, unlock EPERM", pthread_mutex_consistent(m) == EINVAL && pthread_mutex_unlock(m) == EPERM);
    p = fork();
    if (p == 0)
        _exit(lock_secs(1));
    int st;
    waitpid(p, &st, 0);
    check("lock from another process EINVAL", WIFEXITED(st) && WEXITSTATUS(st) == EINVAL);

    printf("robust2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Robust mutexes (POSIX, PTHREAD_MUTEX_ROBUST): when the owner thread or
 * process ends holding one, the next locker gets EOWNERDEAD and owns it;
 * pthread_mutex_consistent() makes it usable again, otherwise unlocking it
 * leaves it ENOTRECOVERABLE. Checked for an owner thread that exits, a
 * waiter already blocked when it does, and an owner process that exits
 * holding a mutex in shared memory; a non-robust mutex stays locked. As
 * procnto: the owner's death is found when a thread next locks the mutex,
 * so a thread already waiting when the owner exits isn't woken by it.
 * Prints one "ok"/"FAIL" line per check and "robusttest: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("robusttest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void checkr(const char *what, int r, int want)
{
    printf("robusttest: %s %s (%d: %s)\n", r == want ? "ok" : "FAIL", what, r, strerror(r));
    fails += r != want;
}

/* Lock, giving up after some seconds (ETIMEDOUT) rather than hanging. */
static int lock_secs(pthread_mutex_t *m, int secs)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_sec += secs;
    return pthread_mutex_timedlock(m, &ts);
}

static int lock(pthread_mutex_t *m)
{
    return lock_secs(m, 1);
}

static void init_robust(pthread_mutex_t *m, int shared)
{
    pthread_mutexattr_t a;
    pthread_mutexattr_init(&a);
    pthread_mutexattr_setrobust(&a, PTHREAD_MUTEX_ROBUST);
    if (shared) {
        pthread_mutexattr_setpshared(&a, PTHREAD_PROCESS_SHARED);
    }
    pthread_mutex_init(m, &a);
    pthread_mutexattr_destroy(&a);
}

static void *lock_and_exit(void *arg)
{
    pthread_mutex_lock(arg);
    return NULL;
}

static volatile int waiter_result = -1;

/* Waits (2 s at most) for the mutex while its owner exits. */
static void *waiter(void *arg)
{
    waiter_result = lock_secs(arg, 2);
    if (waiter_result == 0 || waiter_result == EOWNERDEAD) {
        pthread_mutex_unlock(arg);
    }
    return NULL;
}

static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;

static void *lock_wait_exit(void *arg)
{
    pthread_mutex_lock(arg);
    pthread_mutex_lock(&gate); /* until the main thread lets it go */
    pthread_mutex_unlock(&gate);
    return NULL;
}

int main(void)
{
    pthread_mutex_t m;
    pthread_t t;

    /* 1. The owner thread exits holding it. */
    init_robust(&m, 0);
    pthread_create(&t, NULL, lock_and_exit, &m);
    pthread_join(t, NULL);
    checkr("lock after the owner thread exited: EOWNERDEAD", lock(&m), EOWNERDEAD);
    checkr("pthread_mutex_consistent()", pthread_mutex_consistent(&m), 0);
    checkr("unlock", pthread_mutex_unlock(&m), 0);
    checkr("lock again", lock(&m), 0);
    checkr("consistent on a consistent mutex: EINVAL", pthread_mutex_consistent(&m), EINVAL);
    pthread_mutex_unlock(&m);
    pthread_mutex_destroy(&m);

    /* 2. Unlocked without being made consistent. */
    init_robust(&m, 0);
    pthread_create(&t, NULL, lock_and_exit, &m);
    pthread_join(t, NULL);
    checkr("lock: EOWNERDEAD", lock(&m), EOWNERDEAD);
    checkr("unlock without consistent", pthread_mutex_unlock(&m), 0);
    checkr("lock: ENOTRECOVERABLE", lock(&m), ENOTRECOVERABLE);
    checkr("trylock: ENOTRECOVERABLE", pthread_mutex_trylock(&m), ENOTRECOVERABLE);
    pthread_mutex_destroy(&m);

    /* 3. A waiter blocked when the owner exits. */
    init_robust(&m, 0);
    pthread_t owner, w;
    pthread_mutex_lock(&gate);
    pthread_create(&owner, NULL, lock_wait_exit, &m);
    usleep(20000);
    pthread_create(&w, NULL, waiter, &m);
    usleep(20000);
    pthread_mutex_unlock(&gate);
    pthread_join(owner, NULL);
    pthread_join(w, NULL);
    checkr("a waiter blocked before the owner exited isn't woken by it (as procnto): ETIMEDOUT", waiter_result,
           ETIMEDOUT);
    checkr("the next locker: EOWNERDEAD", lock(&m), EOWNERDEAD);
    pthread_mutex_consistent(&m);
    pthread_mutex_unlock(&m);
    pthread_mutex_destroy(&m);

    /* 4. The owner process exits, the mutex in shared memory. */
    pthread_mutex_t *sm = mmap(NULL, sizeof *sm, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, NOFD, 0);
    init_robust(sm, 1);
    pid_t pid = fork();
    if (pid == 0) {
        pthread_mutex_lock(sm);
        _exit(0);
    }
    waitpid(pid, NULL, 0);
    checkr("lock after the owner process exited: EOWNERDEAD", lock(sm), EOWNERDEAD);
    pthread_mutex_consistent(sm);
    pthread_mutex_unlock(sm);
    checkr("lock again", lock(sm), 0);
    pthread_mutex_unlock(sm);

    /* 5. A non-robust mutex stays locked. */
    pthread_mutex_t plain;
    pthread_mutex_init(&plain, NULL);
    pthread_create(&t, NULL, lock_and_exit, &plain);
    pthread_join(t, NULL);
    checkr("a plain mutex whose owner exited: trylock EBUSY", pthread_mutex_trylock(&plain), EBUSY);

    printf("robusttest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

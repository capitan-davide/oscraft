/* pthread_cancel(): deferred cancellation at cancellation points (sleep,
 * a condition variable, MsgReceive(), sem_wait(), pthread_join(),
 * pthread_testcancel()), with cleanup handlers; a thread of the
 * asynchronous type (QNX 8 cancels it at cancellation points too: one that
 * spins, even through other kernel calls, goes on); a cancel that waits while
 * cancellation is disabled; and none at a non-cancellation point
 * (pthread_mutex_lock()). Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <pthread.h>
#include <semaphore.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;
static volatile int cleaned;

static void check(const char *what, int ok)
{
    printf("canceltest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void cleanup(void *arg)
{
    (void)arg;
    cleaned = 1;
}

/* Start `fn`, let it block, cancel it; did it end canceled, cleanup run? */
static int cancel_case(void *(*fn)(void *), void *arg)
{
    pthread_t t;
    cleaned = 0;
    pthread_create(&t, NULL, fn, arg);
    usleep(100 * 1000);
    pthread_cancel(t);
    void *r = NULL;
    pthread_join(t, &r);
    return r == PTHREAD_CANCELED && cleaned;
}

static void *sleeper(void *arg)
{
    (void)arg;
    pthread_cleanup_push(cleanup, NULL);
    for (;;)
        sleep(10);
    pthread_cleanup_pop(0);
    return NULL;
}

static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t c = PTHREAD_COND_INITIALIZER;

static void unlock(void *arg)
{
    cleaned = 1;
    pthread_mutex_unlock(arg);
}

static void *cond_waiter(void *arg)
{
    (void)arg;
    pthread_mutex_lock(&m);
    pthread_cleanup_push(unlock, &m);
    for (;;)
        pthread_cond_wait(&c, &m);
    pthread_cleanup_pop(0);
    return NULL;
}

static void *receiver(void *arg)
{
    int chid = *(int *)arg;
    char buf[16];
    pthread_cleanup_push(cleanup, NULL);
    for (;;)
        MsgReceive(chid, buf, sizeof buf, NULL);
    pthread_cleanup_pop(0);
    return NULL;
}

static sem_t sem;

static void *sem_waiter(void *arg)
{
    (void)arg;
    pthread_cleanup_push(cleanup, NULL);
    for (;;)
        sem_wait(&sem);
    pthread_cleanup_pop(0);
    return NULL;
}

static void *joiner(void *arg)
{
    pthread_cleanup_push(cleanup, NULL);
    pthread_join(*(pthread_t *)arg, NULL);
    pthread_cleanup_pop(0);
    return NULL;
}

static void *async_sleeper(void *arg)
{
    (void)arg;
    pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, NULL);
    pthread_cleanup_push(cleanup, NULL);
    for (;;)
        sleep(10);
    pthread_cleanup_pop(0);
    return NULL;
}

static volatile int disabled_reached;

static void *disabled(void *arg)
{
    (void)arg;
    pthread_setcancelstate(PTHREAD_CANCEL_DISABLE, NULL);
    pthread_cleanup_push(cleanup, NULL);
    usleep(300 * 1000); /* canceled meanwhile: not a cancellation point now */
    disabled_reached = 1;
    pthread_setcancelstate(PTHREAD_CANCEL_ENABLE, NULL);
    pthread_testcancel();
    disabled_reached = 2;
    pthread_cleanup_pop(0);
    return NULL;
}

static pthread_mutex_t held = PTHREAD_MUTEX_INITIALIZER;
static volatile int locked_through;

static void *locker(void *arg)
{
    (void)arg;
    pthread_cleanup_push(cleanup, NULL);
    pthread_mutex_lock(&held); /* not a cancellation point */
    locked_through = 1;
    pthread_mutex_unlock(&held);
    pthread_testcancel();
    locked_through = 2;
    pthread_cleanup_pop(0);
    return NULL;
}

int main(void)
{
    check("sleep()", cancel_case(sleeper, NULL));
    check("pthread_cond_wait(), mutex held in cleanup", cancel_case(cond_waiter, NULL));
    check("mutex free after the cond_wait cancel", pthread_mutex_trylock(&m) == 0);
    pthread_mutex_unlock(&m);

    int chid = ChannelCreate(0);
    check("MsgReceive()", cancel_case(receiver, &chid));
    ChannelDestroy(chid);

    sem_init(&sem, 0, 0);
    check("sem_wait()", cancel_case(sem_waiter, NULL));

    pthread_t forever;
    pthread_create(&forever, NULL, sleeper, NULL);
    check("pthread_join()", cancel_case(joiner, &forever));
    pthread_cancel(forever);
    pthread_join(forever, NULL);

    check("asynchronous type", cancel_case(async_sleeper, NULL));

    check("disabled, then pthread_testcancel()",
          cancel_case(disabled, NULL) && disabled_reached == 1);

    pthread_mutex_lock(&held);
    pthread_t t;
    cleaned = 0;
    pthread_create(&t, NULL, locker, NULL);
    usleep(100 * 1000);
    pthread_cancel(t);
    usleep(100 * 1000);
    pthread_mutex_unlock(&held);
    void *r = NULL;
    pthread_join(t, &r);
    check("not at pthread_mutex_lock(), at the next point",
          r == PTHREAD_CANCELED && locked_through == 1 && cleaned);

    check("ESRCH for an ended thread", pthread_cancel(t) == ESRCH);

    printf("canceltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

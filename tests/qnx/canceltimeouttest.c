/* A TimerTimeout() armed for a cancellation point that a pending cancel
 * turns into the cancel stub (review item SCHED-16), as procnto
 * (measured): the call isn't made, and the timeout goes with it; the
 * first blocking call of the cleanup handlers waits as usual. The kernel
 * kept the timeout armed on that path, so the handler's sem_wait() failed
 * at once with ETIMEDOUT. Prints one "ok"/"FAIL" line per check and
 * "canceltimeouttest: passed". */
#include <errno.h>
#include <pthread.h>
#include <semaphore.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

static int fails;
static sem_t s;
static volatile int handler_returned;

static void check(const char *what, int ok)
{
    printf("canceltimeouttest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void handler(void *a)
{
    (void)a;
    sem_wait(&s);
    handler_returned = 1;
}

static void *body(void *a)
{
    (void)a;
    int chid = ChannelCreate(0);
    pthread_cleanup_push(handler, NULL);
    pthread_cancel(pthread_self());
    /* a timeout that has expired by the time anything blocks */
    uint64_t ns = 1;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE | _NTO_TIMEOUT_SEM, NULL, &ns, NULL);
    char buf[16];
    MsgReceive(chid, buf, sizeof buf, NULL);
    pthread_cleanup_pop(0);
    return NULL;
}

int main(void)
{
    sem_init(&s, 0, 0);
    pthread_t t;
    pthread_create(&t, NULL, body, NULL);
    usleep(100000);
    check("the cleanup handler's sem_wait() still waits after 100 ms", !handler_returned);
    sem_post(&s);
    void *ret = NULL;
    pthread_join(t, &ret);
    check("the thread ended cancelled", ret == PTHREAD_CANCELED);
    printf("canceltimeouttest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

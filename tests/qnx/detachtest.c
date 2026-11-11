/* Joining and detaching one thread at once (review item SCHED-11), as
 * procnto (measured): a thread blocked in pthread_join() returns EINVAL
 * at once when its target is detached, and of a join and a detach racing
 * on an ended thread exactly one takes it. The kernel used to leave the
 * joiner blocked and then let it take the detached thread (0), so both
 * the join and the thread's end freed its ID; and a join that passed its
 * check before a detach still succeeded, nearly every time in the race.
 * Prints one "ok"/"FAIL" line per check and "detachtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

#define ROUNDS 2000

static int fails;
static pthread_t target;
static volatile int release_target, join_done, go, ready;
static int join_r = -1, detach_r;

static void check(const char *what, int ok)
{
    printf("detachtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *body(void *a)
{
    (void)a;
    while (!release_target)
        usleep(1000);
    return NULL;
}

static void *joiner(void *a)
{
    (void)a;
    join_r = pthread_join(target, NULL);
    join_done = 1;
    return NULL;
}

static void *quick(void *a)
{
    (void)a;
    return NULL;
}

static void *detacher(void *a)
{
    (void)a;
    for (;;) {
        while (!go)
            ;
        go = 0;
        detach_r = pthread_detach(target);
        __atomic_store_n(&ready, 1, __ATOMIC_RELEASE);
    }
    return NULL;
}

int main(void)
{
    /* a blocked joiner, then a detach */
    pthread_create(&target, NULL, body, NULL);
    pthread_t j;
    pthread_create(&j, NULL, joiner, NULL);
    usleep(50000);
    check("detach of a thread being joined", pthread_detach(target) == 0);
    usleep(50000);
    check("the joiner returned at once", join_done);
    check("with EINVAL", join_r == EINVAL);
    release_target = 1;
    usleep(50000);
    check("and still EINVAL once the thread ended", join_done && join_r == EINVAL);
    pthread_join(j, NULL);

    /* ended, then detached and joined in either order */
    pthread_t q;
    pthread_create(&q, NULL, quick, NULL);
    usleep(20000);
    check("detach of an ended thread", pthread_detach(q) == 0);
    check("then join: ESRCH", pthread_join(q, NULL) == ESRCH);
    pthread_create(&q, NULL, quick, NULL);
    usleep(20000);
    check("join of an ended thread", pthread_join(q, NULL) == 0);
    check("then detach: ESRCH", pthread_detach(q) == ESRCH);

    /* a join and a detach racing on an ended thread */
    pthread_t d;
    pthread_create(&d, NULL, detacher, NULL);
    int both = 0, neither = 0, odd = 0;
    for (int i = 0; i < ROUNDS; i++) {
        pthread_create(&target, NULL, quick, NULL);
        for (volatile int k = 0; k < 20000; k++)
            ;
        ready = 0;
        go = 1;
        int r = pthread_join(target, NULL);
        while (!__atomic_load_n(&ready, __ATOMIC_ACQUIRE))
            ;
        both += r == 0 && detach_r == 0;
        neither += r != 0 && detach_r != 0;
        odd += (r != 0 && r != EINVAL && r != ESRCH) || (detach_r != 0 && detach_r != EINVAL && detach_r != ESRCH);
    }
    printf("detachtest: %d rounds: both took it %d, neither %d, other errors %d\n", ROUNDS, both, neither, odd);
    check("racing join and detach: exactly one takes the thread", both == 0 && neither == 0 && odd == 0);

    /* thread IDs are still unique */
    release_target = 0;
    pthread_t a, b;
    pthread_create(&a, NULL, body, NULL);
    pthread_create(&b, NULL, body, NULL);
    check("two new threads have distinct IDs", a != b);
    release_target = 1;
    pthread_join(a, NULL);
    pthread_join(b, NULL);
    printf("detachtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

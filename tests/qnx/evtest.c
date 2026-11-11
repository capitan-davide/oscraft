/* SIGEV_THREAD events (as procnto): each expiry of a timer runs its
 * function in a new thread with the event's value; the attributes give
 * the thread's priority; without PROCMGR_AID_SIGEV_THREAD the event can't
 * be registered. Prints one "ok"/"FAIL" line per check and
 * "evtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("evtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static volatile int calls, value, prio;
static volatile pthread_t who;

static void notify(union sigval v)
{
    struct sched_param sp;
    int policy;
    pthread_getschedparam(pthread_self(), &policy, &sp);
    prio = sp.sched_priority;
    value = v.sival_int;
    who = pthread_self();
    calls++;
}

/* Run a 20 ms periodic SIGEV_THREAD timer for 150 ms. */
static int run_timer(pthread_attr_t *attr)
{
    struct sigevent ev;
    memset(&ev, 0, sizeof ev);
    SIGEV_THREAD_INIT(&ev, notify, 77, attr);
    timer_t t;
    if (timer_create(CLOCK_MONOTONIC, &ev, &t) != 0) {
        return errno;
    }
    calls = 0;
    struct itimerspec its = { .it_value = { 0, 20000000 }, .it_interval = { 0, 20000000 } };
    timer_settime(t, 0, &its, NULL);
    usleep(150000);
    timer_delete(t);
    usleep(30000);
    return 0;
}

static int denied(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_SIGEV_THREAD, PROCMGR_AID_EOL);
    return run_timer(NULL);
}

int main(void)
{
    check("timer_create() with SIGEV_THREAD", run_timer(NULL) == 0);
    check("its function runs at each expiry", calls >= 5);
    check("with the event's value", value == 77);
    check("in a thread of its own", who != 0 && who != pthread_self());

    pthread_attr_t a;
    struct sched_param sp = { .sched_priority = 40 };
    pthread_attr_init(&a);
    pthread_attr_setinheritsched(&a, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&a, SCHED_RR);
    pthread_attr_setschedparam(&a, &sp);
    run_timer(&a);
    check("the attributes give its priority", calls >= 5 && prio == 40);

    pid_t pid = fork();
    if (pid == 0) {
        _exit(denied());
    }
    int st;
    waitpid(pid, &st, 0);
    check("without PROCMGR_AID_SIGEV_THREAD: EPERM", WIFEXITED(st) && WEXITSTATUS(st) == EPERM);

    printf("evtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

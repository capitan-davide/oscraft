/* Wakeup latency across CPUs: a thread pinned to CPU 1 waits on a
 * semaphore that a thread on CPU 0 posts; how long from the post until it
 * runs, with CPU 1 idle and with CPU 1 busy at a lower priority (the
 * waiter must preempt it). Without an interprocessor interrupt the other
 * CPU only notices at its next clock tick (up to 1 ms). Prints the mean
 * and the worst of each, then one "ok"/"FAIL" line per case. */
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <unistd.h>

#define ROUNDS 200

static int fails;
static sem_t go;
static volatile uint64_t posted, woke;
static volatile int spin = 1;

static void check(const char *what, int ok)
{
    printf("ipitest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void pin(int cpu)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
}

static void *waiter(void *arg)
{
    (void)arg;
    pin(1);
    for (int i = 0; i < ROUNDS; i++) {
        sem_wait(&go);
        woke = ClockCycles();
    }
    return NULL;
}

static void *spinner(void *arg)
{
    (void)arg;
    pin(1);
    while (spin)
        ;
    return NULL;
}

static pthread_t start(void *(*fn)(void *), int prio)
{
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setinheritsched(&a, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&a, SCHED_FIFO);
    struct sched_param sp = {.sched_priority = prio};
    pthread_attr_setschedparam(&a, &sp);
    pthread_t t;
    pthread_create(&t, &a, fn, NULL);
    return t;
}

/* Mean and worst latency (us) of ROUNDS wakeups. */
static void measure(double *mean, double *worst)
{
    double cps = (double)SYSPAGE_ENTRY(qtime)->cycles_per_sec;
    sem_init(&go, 0, 0);
    pthread_t w = start(waiter, 30);
    usleep(20000);
    double sum = 0;
    *worst = 0;
    for (int i = 0; i < ROUNDS; i++) {
        woke = 0;
        posted = ClockCycles();
        sem_post(&go);
        while (woke == 0)
            sched_yield();
        double us = (double)(woke - posted) * 1e6 / cps;
        sum += us;
        *worst = us > *worst ? us : *worst;
        usleep(1500); /* not in step with the tick */
    }
    pthread_join(w, NULL);
    *mean = sum / ROUNDS;
}

int main(void)
{
    pin(0);
    struct sched_param sp = {.sched_priority = 20};
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    if (_syspage_ptr->num_cpu < 2) {
        printf("ipitest: one CPU only\nipitest: passed\n");
        return 0;
    }
    double idle_mean, idle_worst, busy_mean, busy_worst;
    measure(&idle_mean, &idle_worst);
    pthread_t s = start(spinner, 5);
    usleep(20000);
    measure(&busy_mean, &busy_worst);
    spin = 0;
    pthread_join(s, NULL);
    printf("ipitest: CPU 1 idle: mean %.0f us, worst %.0f us\n", idle_mean, idle_worst);
    printf("ipitest: CPU 1 busy at a lower priority: mean %.0f us, worst %.0f us\n", busy_mean,
           busy_worst);
    check("an idle CPU runs a woken thread at once", idle_mean < 200);
    check("a busy CPU is preempted at once", busy_mean < 200);
    printf("ipitest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

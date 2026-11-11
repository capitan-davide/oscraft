/* A timer armed to expire at the end of time (it_value.tv_sec -1, a
 * deadline at the top of the range) on each CPU in turn, while that CPU
 * idles 20 times with no other deadline (it joins, without timeout, a
 * thread spinning on another CPU). procnto accepts it and goes on (issue
 * #282, measured); a debug OsCraft kernel rounded the deadline up to a
 * tick past the top and panicked. Prints a line per CPU and
 * "endtimetest: passed". */
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <time.h>

static int other, fails;

static long long ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void *spinner(void *a)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << other));
    long long end = ns() + 20000000;
    while (ns() < end) {
    }
    return a;
}

int main(void)
{
    int n = _syspage_ptr->num_cpu;
    struct sigevent ev;
    SIGEV_NONE_INIT(&ev);
    for (int c = 0; c < n; c++) {
        other = (c + 1) % n;
        ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << c));
        timer_t t;
        timer_create(CLOCK_MONOTONIC, &ev, &t);
        struct itimerspec it = { { 0, 1000000 }, { -1, 0 } };
        int r = timer_settime(t, 0, &it, NULL);
        for (int i = 0; i < 20; i++) {
            pthread_t w;
            pthread_create(&w, NULL, spinner, NULL);
            pthread_join(w, NULL);
        }
        timer_delete(t);
        fails += r != 0;
        printf("endtimetest: %s cpu %d: armed (%d), idled 20 times\n", r == 0 ? "ok" : "FAIL", c, r);
        fflush(stdout);
    }
    printf("endtimetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

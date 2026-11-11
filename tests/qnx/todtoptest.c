/* A CLOCK_REALTIME timer when the time of day is near the top of the
 * 64-bit nanosecond range (review item SCHED-26), as procnto (measured):
 * root sets it there with ClockTime(); a relative 60 s timer has 59 or 60
 * s left (timer_gettime(): the tick rounds it), and TimerInfo()'s next
 * expiry, the time of day plus 60 s, wraps around below the clock. The
 * kernel added the time-of-day offset with a plain +, which panicked the
 * debug kernel. The clock is put back at once. Prints one "ok"/"FAIL"
 * line per check and "todtoptest: passed". */
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <time.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("todtoptest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    uint64_t saved, now;
    ClockTime(CLOCK_REALTIME, NULL, &saved);
    uint64_t top = UINT64_MAX - 10000000000ULL;
    check("ClockTime() to 10 s below the top", ClockTime(CLOCK_REALTIME, &top, NULL) != -1);
    ClockTime(CLOCK_REALTIME, NULL, &now);
    struct sigevent ev;
    SIGEV_NONE_INIT(&ev);
    timer_t t;
    timer_create(CLOCK_REALTIME, &ev, &t);
    struct itimerspec it = { .it_value = { 60, 0 } };
    check("timer_settime(relative 60 s)", timer_settime(t, 0, &it, NULL) == 0);
    struct itimerspec g;
    memset(&g, 0, sizeof g);
    int r = timer_gettime(t, &g);
    struct _timer_info info;
    memset(&info, 0, sizeof info);
    int ri = TimerInfo(0, t, 0, &info);
    timer_delete(t);
    ClockTime(CLOCK_REALTIME, &saved, NULL);
    check("timer_gettime(): 59 or 60 s left", r == 0 && g.it_value.tv_sec >= 59 && g.it_value.tv_sec <= 60);
    check("TimerInfo()", ri != -1);
    check("its next expiry wraps around below the clock", info.itime.nsec < now);
    check("and 59 or 60 s left", info.otime.nsec / 1000000000ULL >= 59 && info.otime.nsec / 1000000000ULL <= 60);
    printf("todtoptest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

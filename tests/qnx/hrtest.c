/* Timer resolution, as in QNX 8 (tickless, with "virtual" ticks of the
 * clock period): a normal periodic timer of 250 us fires on ticks (1 ms
 * apart); a high-resolution timer (TIMER_TOLERANCE below the tick) fires
 * every 250 us; nanosleep(100 us) ends on a tick. Prints the measured
 * times, then one "ok"/"FAIL" line per case. `hrtest -v` only prints. */
#include <stdio.h>
#include <string.h>
#include <stdint.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <time.h>
#include <unistd.h>

#define PERIOD_NS 250000
#define EXPIRIES 400

static int fails, verbose_only;

static void check(const char *what, int ok)
{
    if (verbose_only)
        return;
    printf("hrtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static long long now_ns(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

static long long now_real(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return ts.tv_sec * 1000000000LL + ts.tv_nsec;
}

/* Mean interval (ns) between EXPIRIES pulses of a periodic timer. */
static long long periodic(int high_res)
{
    int chid = ChannelCreate(0);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, coid, SIGEV_PULSE_PRIO_INHERIT, 1, 0);
    timer_t t;
    timer_create(CLOCK_MONOTONIC, &ev, &t);
    if (high_res) {
        struct itimerspec tol;
        memset(&tol, 0, sizeof tol);
        tol.it_value.tv_nsec = 1;
        if (timer_settime(t, TIMER_TOLERANCE, &tol, NULL) != 0)
            printf("hrtest: TIMER_TOLERANCE refused\n");
    }
    struct itimerspec spec;
    memset(&spec, 0, sizeof spec);
    spec.it_value.tv_nsec = PERIOD_NS;
    spec.it_interval.tv_nsec = PERIOD_NS;
    timer_settime(t, 0, &spec, NULL);
    struct _pulse p;
    MsgReceivePulse(chid, &p, sizeof p, NULL); /* first, to start timing */
    long long t0 = now_ns();
    for (int i = 0; i < EXPIRIES; i++)
        MsgReceivePulse(chid, &p, sizeof p, NULL);
    long long mean = (now_ns() - t0) / EXPIRIES;
    timer_delete(t);
    ConnectDetach(coid);
    ChannelDestroy(chid);
    return mean;
}

int main(int argc, char **argv)
{
    verbose_only = argc > 1 && !strcmp(argv[1], "-v");
    struct timespec res;
    clock_getres(CLOCK_MONOTONIC, &res);
    struct _clockperiod period;
    ClockPeriod(CLOCK_MONOTONIC, NULL, &period, 0);

    long long normal = periodic(0);
    long long high = periodic(1);

    struct timespec nap = {0, 100000};
    long long t0 = now_ns();
    for (int i = 0; i < 100; i++)
        nanosleep(&nap, NULL);
    long long sleep = (now_ns() - t0) / 100;

    /* The process's default tolerance (procmgr_timer_tolerance()) doesn't
     * make a timer high-resolution, as procnto: only its own
     * TIMER_TOLERANCE does. */
    uint64_t tol = 100000, none = 0;
    procmgr_timer_tolerance(0, &tol, NULL);
    long long dflt = periodic(0);
    t0 = now_ns();
    for (int i = 0; i < 100; i++)
        nanosleep(&nap, NULL);
    long long dsleep = (now_ns() - t0) / 100;
    procmgr_timer_tolerance(0, &none, NULL);

    printf("hrtest: tick %lu ns, clock_getres %ld ns\n", period.nsec, res.tv_nsec);
    printf("hrtest: 250 us timer: normal every %lld us, high-resolution every %lld us\n",
           normal / 1000, high / 1000);
    printf("hrtest: nanosleep(100 us) takes %lld us\n", sleep / 1000);
    printf("hrtest: with a default tolerance of 100 us: the timer every %lld us, nanosleep(100 us) %lld us\n",
           dflt / 1000, dsleep / 1000);

    check("a normal timer fires on ticks", normal >= 900000 && normal <= 1100000);
    check("a high-resolution timer fires every 250 us", high >= 225000 && high <= 275000);
    check("nanosleep(100 us) ends on a tick", sleep >= 900000 && sleep <= 2100000);
    check("a default tolerance below the tick: still on ticks", dflt >= 900000 && dflt <= 1100000);
    /* Setting the time of day (as procnto): before the boot it becomes the
     * boot time, not a negative offset (which libc reads as the end of
     * time); a real date is kept. Set back afterwards. */
    struct timespec saved, ts = {1, 0};
    clock_gettime(CLOCK_REALTIME, &saved);
    clock_settime(CLOCK_REALTIME, &ts);
    long long early = now_real() - now_ns();
    ts.tv_sec = 1700000000;
    clock_settime(CLOCK_REALTIME, &ts);
    long long dated = now_real() / 1000000000LL;
    clock_settime(CLOCK_REALTIME, &saved);
    check("clock_settime() before the boot: the boot time", early > -1000000 && early < 1000000);
    check("clock_settime() to a date", dated >= 1700000000 && dated <= 1700000001);
    if (!verbose_only)
        printf("hrtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

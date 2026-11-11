/* Clocks and timers as procnto gives them:
 * - clock_gettime()/clock_getres()/clock_settime() on each clock;
 *   CLOCK_MONOTONIC counts from qtime's boot_cc;
 *   CPU-time clocks of the caller's own process and threads may be set
 *   (another process's: EPERM);
 * - nanosleep()/clock_nanosleep() errors (a CPU-time clock: ENOTSUP for a
 *   process's, EINVAL for a thread's);
 * - timers on CPU-time clocks: one fires once the process has run long
 *   enough, not while it sleeps; periodic ones; a thread's;
 * - a disarmed timer keeps no interval;
 * - timer_gettime(), deleted timers, setitimer(), alarm();
 * - a timer's signals: one pending at a time, the other expiries counted
 *   as overruns (until timer_getoverrun()); taken by sigwaitinfo(), the
 *   expiry after still counts one and the next sends it again; a thread
 *   waiting for it gets each one.
 * Prints one "ok"/"FAIL" line per check and "clocktest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

static int fails;
static volatile int fired;

static void check(const char *what, int ok)
{
    printf("clocktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* `call` (returning -1 and errno on failure, or an error number) gives
 * `ret` and, if it failed, errno `err`. */
#define C(call, ret, err)                                                                              \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(call);                                                                        \
        printf("clocktest: %s %s (%ld, errno %d)\n", r_ == (ret) && (r_ != -1 || errno == (err)) ? "ok" : "FAIL", \
               #call, r_, errno);                                                                      \
        fails += !(r_ == (ret) && (r_ != -1 || errno == (err)));                                       \
    } while (0)

static void on_alarm(int signo)
{
    (void)signo;
    fired++;
}

static void clocks(void)
{
    struct timespec ts, res;
    clockid_t cid;
    C(clock_gettime(CLOCK_REALTIME, &ts), 0, 0);
    C(clock_gettime(CLOCK_MONOTONIC, &ts), 0, 0);
    C(clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &ts), 0, 0);
    C(clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts), 0, 0);
    C(clock_gettime(99, &ts), -1, EINVAL);
    {
        struct qtime_entry *q = SYSPAGE_ENTRY(qtime);
        uint64_t cc = ClockCycles();
        clock_gettime(CLOCK_MONOTONIC, &ts);
        double mono = ts.tv_sec + ts.tv_nsec / 1e9, from_boot = (double)(cc - q->boot_cc) / q->cycles_per_sec;
        check("CLOCK_MONOTONIC counts from qtime's boot_cc (the kernel's start)",
              q->boot_cc != 0 && mono - from_boot < 0.001 && from_boot - mono < 0.001);
    }
    C(clock_gettime(CLOCK_SOFTTIME, &ts), 0, 0);
    clock_getres(CLOCK_REALTIME, &res);
    check("CLOCK_REALTIME's resolution: the tick", res.tv_sec == 0 && res.tv_nsec == 1000000);
    clock_getres(CLOCK_PROCESS_CPUTIME_ID, &res);
    check("a CPU-time clock's resolution: the tick", res.tv_nsec == 1000000);
    C(clock_getres(99, &res), -1, EINVAL);
    C(clock_getcpuclockid(getpid(), &cid), 0, 0);
    C(clock_getcpuclockid(99999, &cid), ESRCH, 0);
    C(pthread_getcpuclockid(pthread_self(), &cid), 0, 0);
    C(clock_gettime(cid, &ts), 0, 0);

    struct timespec zero = { 0, 0 }, big = { 100, 0 }, after;
    C(clock_settime(CLOCK_MONOTONIC, &zero), -1, EINVAL);
    struct timespec bad = { 0, 2000000000 };
    C(clock_settime(CLOCK_REALTIME, &bad), -1, EINVAL);
    C(clock_settime(CLOCK_PROCESS_CPUTIME_ID, &zero), 0, 0);
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &after);
    check("the process's CPU clock set to 0 reads from 0", after.tv_sec == 0 && after.tv_nsec < 50000000);
    C(clock_settime(CLOCK_PROCESS_CPUTIME_ID, &big), 0, 0);
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &after);
    check("and set to 100 s reads from there", after.tv_sec == 100);
    C(clock_settime(CLOCK_THREAD_CPUTIME_ID, &zero), 0, 0);
    clock_getcpuclockid(1, &cid);
    C(clock_settime(cid, &zero), -1, EPERM);
}

static void sleeps(void)
{
    struct timespec n = { 0, -1 };
    C(nanosleep(&n, NULL), -1, EINVAL);
    n.tv_nsec = 1000000000;
    C(nanosleep(&n, NULL), -1, EINVAL);
    n.tv_nsec = 1000;
    C(clock_nanosleep(CLOCK_PROCESS_CPUTIME_ID, 0, &n, NULL), ENOTSUP, 0);
    C(clock_nanosleep(99, 0, &n, NULL), EINVAL, 0);
    C(clock_nanosleep(CLOCK_THREAD_CPUTIME_ID, 0, &n, NULL), EINVAL, 0);
    struct timespec past;
    clock_gettime(CLOCK_MONOTONIC, &past);
    past.tv_sec -= 1;
    C(clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &past, NULL), 0, 0);
}

/* Spins for `ns` of monotonic time, or until a timer fires. */
static void spin(long ns)
{
    struct timespec start, now;
    clock_gettime(CLOCK_MONOTONIC, &start);
    do {
        clock_gettime(CLOCK_MONOTONIC, &now);
    } while ((now.tv_sec - start.tv_sec) * 1000000000L + now.tv_nsec - start.tv_nsec < ns);
}

static void cpu_timer(void)
{
    signal(SIGUSR1, on_alarm);
    struct sigevent ev;
    SIGEV_SIGNAL_INIT(&ev, SIGUSR1);
    timer_t t;
    C(timer_create(CLOCK_PROCESS_CPUTIME_ID, &ev, &t), 0, 0);
    struct itimerspec it = { .it_value = { 0, 50000000 } }, cur;
    timer_settime(t, 0, &it, NULL);
    fired = 0;
    usleep(200000);
    check("a CPU-time timer doesn't fire while the process sleeps", fired == 0);
    for (int i = 0; i < 300 && fired == 0; i++) {
        spin(10000000);
    }
    check("it fires once the process has run 50 ms", fired == 1);
    it.it_value.tv_nsec = it.it_interval.tv_nsec = 10000000;
    fired = 0;
    timer_settime(t, 0, &it, NULL);
    spin(105000000);
    printf("clocktest:   periodic CPU-time timer: fired %d in 105 ms\n", fired);
    check("a periodic one fires every 10 ms of CPU time", fired >= 9 && fired <= 11);
    struct itimerspec off = { .it_interval = { 0, 7000000 } };
    timer_settime(t, 0, &off, &cur);
    check("disarmed: the old interval returned", cur.it_interval.tv_nsec == 10000000);
    timer_gettime(t, &cur);
    check("and no interval kept", cur.it_value.tv_nsec == 0 && cur.it_interval.tv_nsec == 0);
    timer_delete(t);
    C(timer_create(CLOCK_THREAD_CPUTIME_ID, &ev, &t), 0, 0);
    it.it_value.tv_nsec = 30000000;
    it.it_interval.tv_nsec = 0;
    fired = 0;
    timer_settime(t, 0, &it, NULL);
    spin(60000000);
    check("a thread's CPU-time timer fires", fired == 1);
    timer_delete(t);
    C(timer_create(99, &ev, &t), -1, EINVAL);
}

static void timers(void)
{
    struct sigevent ev;
    SIGEV_NONE_INIT(&ev);
    timer_t t;
    C(timer_create(CLOCK_MONOTONIC, &ev, &t), 0, 0);
    struct itimerspec it = { { 0, 1000000 }, { 0, 1000000 } }, cur;
    C(timer_settime(t, 0, &it, NULL), 0, 0);
    usleep(30000);
    timer_gettime(t, &cur);
    check("timer_gettime(): the interval", cur.it_interval.tv_nsec == 1000000);
    it.it_value.tv_nsec = 2000000000;
    C(timer_settime(t, 0, &it, NULL), -1, EINVAL);
    it.it_value.tv_sec = -1;
    it.it_value.tv_nsec = 0;
    C(timer_settime(t, 0, &it, NULL), 0, 0);
    C(timer_delete(t), 0, 0);
    C(timer_delete(t), -1, EINVAL);
    C(timer_getoverrun(t), -1, EINVAL);
    C(timer_gettime(t, &cur), -1, EINVAL);
    struct itimerval iv = { { 0, 0 }, { 0, 0 } }, oiv;
    C(setitimer(ITIMER_REAL, &iv, &oiv), 0, 0);
    C(setitimer(ITIMER_VIRTUAL, &iv, &oiv), -1, EINVAL);
    C(setitimer(99, &iv, &oiv), -1, EINVAL);
    C(getitimer(ITIMER_REAL, &oiv), 0, 0);
    C(alarm(0), 0, 0);
    C(alarm(100), 0, 0);
    C(alarm(0), 99, 0);
}

/* Sleep until `ms` after `t0` (CLOCK_MONOTONIC). */
static void until(const struct timespec *t0, long ms)
{
    struct timespec t = *t0;
    t.tv_nsec += ms * 1000000;
    t.tv_sec += t.tv_nsec / 1000000000;
    t.tv_nsec %= 1000000000;
    clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &t, NULL);
}

static int overruns(timer_t t)
{
    struct _timer_info ti;
    TimerInfo(0, t, 0, &ti);
    return ti.overruns;
}

static int pending(int signo)
{
    sigset_t set;
    sigpending(&set);
    return sigismember(&set, signo);
}

static void sa_siginfo(int s, siginfo_t *i, void *c)
{
    (void)s;
    (void)i;
    (void)c;
}

static void overrun_counts(void)
{
    /* SA_SIGINFO would queue each signal sent: a timer's still don't. */
    struct sigaction sa = {.sa_sigaction = sa_siginfo, .sa_flags = SA_SIGINFO};
    sigaction(SIGUSR2, &sa, NULL);
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR2);
    sigprocmask(SIG_BLOCK, &set, NULL);
    struct sigevent ev;
    SIGEV_SIGNAL_VALUE_INIT(&ev, SIGUSR2, 77);
    timer_t t;
    timer_create(CLOCK_MONOTONIC, &ev, &t);
    struct itimerspec it = {{0, 10000000}, {0, 10000000}}, stop = {{0, 0}, {0, 0}};
    struct timespec t0;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    timer_settime(t, 0, &it, NULL);
    /* Expiries at 10, 20, ... ms. */
    until(&t0, 35);
    int o35 = overruns(t);
    siginfo_t si;
    struct timespec zero = {0, 0};
    int took = sigtimedwait(&set, &si, &zero);
    until(&t0, 45);
    int o45 = overruns(t), p45 = pending(SIGUSR2);
    until(&t0, 55);
    int o55 = overruns(t), p55 = pending(SIGUSR2);
    until(&t0, 105);
    timer_settime(t, 0, &stop, NULL);
    int o105 = overruns(t);
    int n = 0;
    while (sigtimedwait(&set, &si, &zero) == SIGUSR2) {
        n++;
    }
    printf("clocktest:   overruns %d at 35 ms, %d at 45, %d at 55, %d at 105; %d signal(s) queued\n", o35, o45, o55,
           o105, n);
    check("a timer's signal pending: expiries count overruns", o35 == 2 && took == SIGUSR2);
    check("taken by sigwaitinfo(): the next expiry counts one more", o45 == 3 && !p45);
    check("the one after sends the signal again", o55 == 3 && p55);
    check("one signal queued, overruns add up", n == 1 && o105 == 8);
    check("timer_getoverrun() reads and resets them", timer_getoverrun(t) == 8 && overruns(t) == 0);

    /* A thread waiting for the signal gets each expiry's. */
    clock_gettime(CLOCK_MONOTONIC, &t0);
    timer_settime(t, 0, &it, NULL);
    for (n = 0; n < 5; n++) {
        sigwaitinfo(&set, &si);
    }
    timer_settime(t, 0, &stop, NULL);
    struct timespec t1;
    clock_gettime(CLOCK_MONOTONIC, &t1);
    long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
    check("sigwaitinfo() waiting: a signal every expiry, no overruns", ms >= 45 && ms <= 60 && overruns(t) == 0);
    timer_delete(t);
    sigprocmask(SIG_UNBLOCK, &set, NULL);
}

int main(void)
{
    clocks();
    sleeps();
    cpu_timer();
    timers();
    overrun_counts();
    printf("clocktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

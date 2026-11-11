/* CPU offlining with procnto's default clock priority (254), as procnto:
 * - SCHED_OFFLINING (and SCHED_HIGH_PRIORITY_IST) threads go up to 254,
 *   the normal policies to 253; SCHED_OFFLINING only for the caller,
 *   with the runstate ability; SCHED_HIGH_PRIORITY_IST only for an
 *   interrupt thread; SchedGet() reports the policy;
 * - TimerDelegate()'s checks, in procnto's order (its success needs a
 *   priority above the clock's: procnto -C0,253);
 * - SchedCtl(SCHED_PROCESSOR_OFFLINE/ONLINE): only SCHED_OFFLINING, with
 *   a runmask of one CPU, EAGAIN for the state it is in; offline, the CPU
 *   is left out of SCHED_PROCESSORS_STATUS, and its other threads run
 *   once it is back;
 * - a thread at the clock's priority holds back its CPU's timers: timers
 *   (high-resolution or not) and timeouts armed there fire when the
 *   priority drops, a periodic timer once then a period later, with no
 *   overruns; another CPU's fire on time.
 * Needs 2 CPUs. Prints one "ok"/"FAIL" line per check and
 * "offlinetest: passed". */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <semaphore.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;
static uint64_t cps;

static void check(const char *what, int ok)
{
    printf("offlinetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int setp(int pol, int prio)
{
    struct sched_param p;
    memset(&p, 0, sizeof p);
    p.sched_priority = prio;
    return SchedSet_r(0, 0, pol, &p);
}

static int pin(int cpu)
{
    unsigned m = cpu < 0 ? ~0u : 1u << cpu;
    return ThreadCtl_r(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)m);
}

static uint64_t ms_since(uint64_t c0)
{
    return (ClockCycles() - c0) * 1000 / cps;
}

static void spin_ms(int n)
{
    uint64_t end = ClockCycles() + cps * n / 1000;
    while (ClockCycles() < end) {
    }
}

static void *idle_thread(void *a)
{
    (void)a;
    pause();
    return NULL;
}

/* A FIFO 20 thread pinned to CPU 1, counting. */
static atomic_long counter;
static atomic_int stop;
static void *counter_thread(void *a)
{
    (void)a;
    pin(1);
    setp(SCHED_FIFO, 20);
    while (!stop) {
        counter++;
    }
    return NULL;
}

/* On CPU 1, at SCHED_OFFLINING 254: offline for 200 ms (no context
 * switch meanwhile: procnto doesn't survive one). */
static int r_off, r_off2, r_on, r_on2;
static long during, status_online;
static uint64_t status_mask;
static void *offliner(void *a)
{
    (void)a;
    /* Above the counter before it goes to its CPU. */
    setp(SCHED_OFFLINING, 254);
    pin(1);
    r_off = SchedCtl_r(SCHED_PROCESSOR_OFFLINE, NULL, 0);
    r_off2 = SchedCtl_r(SCHED_PROCESSOR_OFFLINE, NULL, 0);
    struct _sched_processors_status s;
    SchedCtl_r(SCHED_PROCESSORS_STATUS, &s, sizeof s);
    status_online = s.num_processors_online;
    status_mask = s.processors_online[0];
    long before = counter;
    spin_ms(200);
    during = counter - before;
    r_on = SchedCtl_r(SCHED_PROCESSOR_ONLINE, NULL, 0);
    r_on2 = SchedCtl_r(SCHED_PROCESSOR_ONLINE, NULL, 0);
    /* Off CPU 1 first: the counter takes it back. */
    pin(0);
    setp(SCHED_FIFO, 10);
    return NULL;
}

/* On CPU 1 at SCHED_OFFLINING 254 for 400 ms. */
static uint64_t lowered;
static void *holder(void *a)
{
    uint64_t c0 = *(uint64_t *)a;
    pin(1);
    setp(SCHED_OFFLINING, 254);
    spin_ms(400);
    setp(SCHED_FIFO, 10);
    lowered = ms_since(c0);
    return NULL;
}

/* Arms a 100 ms timeout on CPU 1 (then may run anywhere). */
static sem_t never;
static uint64_t timed_out;
static void *waiter(void *a)
{
    uint64_t c0 = *(uint64_t *)a;
    pin(1);
    pin(-1);
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    ts.tv_nsec += 100000000;
    ts.tv_sec += ts.tv_nsec / 1000000000;
    ts.tv_nsec %= 1000000000;
    if (sem_timedwait_monotonic(&never, &ts) == -1 && errno == ETIMEDOUT) {
        timed_out = ms_since(c0);
    }
    return NULL;
}

/* Timers and a timeout armed on CPU 1 wait for the thread at the clock's
 * priority there; a timer armed on CPU 0 doesn't. */
static void held_timers(int chid, int coid)
{
    struct sigevent ev;
    timer_t on1, on0, every;
    SIGEV_PULSE_INIT(&ev, coid, 10, 1, 0);
    timer_create(CLOCK_MONOTONIC, &ev, &on1);
    SIGEV_PULSE_INIT(&ev, coid, 10, 2, 0);
    timer_create(CLOCK_MONOTONIC, &ev, &on0);
    SIGEV_PULSE_INIT(&ev, coid, 10, 3, 0);
    timer_create(CLOCK_MONOTONIC, &ev, &every);
    sem_init(&never, 0, 0);
    uint64_t c0 = ClockCycles();
    pthread_t w, h;
    pthread_create(&w, NULL, waiter, &c0);
    usleep(10000);
    struct itimerspec once = {{0, 100000000}, {0, 0}}, tick = {{0, 10000000}, {0, 10000000}};
    pin(1);
    timer_settime(on1, 0, &once, NULL);
    timer_settime(every, 0, &tick, NULL);
    pin(0);
    timer_settime(on0, 0, &once, NULL);
    lowered = 0;
    pthread_create(&h, NULL, holder, &c0);
    uint64_t t1 = 0, t0 = 0, p1 = 0, p2 = 0;
    while (!t1 || !t0 || !p2) {
        struct _pulse pl;
        uint64_t to = 2000000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &to, NULL);
        if (MsgReceivePulse(chid, &pl, sizeof pl, NULL) == -1) {
            break;
        }
        uint64_t now = ms_since(c0);
        if (pl.code == 1) {
            t1 = now;
        } else if (pl.code == 2) {
            t0 = now;
        } else if (!p1) {
            p1 = now;
        } else if (!p2) {
            p2 = now;
        }
    }
    pthread_join(h, NULL);
    pthread_join(w, NULL);
    struct _timer_info ti;
    TimerInfo(0, every, 0, &ti);
    printf("offlinetest:   lowered at %llu ms: CPU 1's timer %llu, timeout %llu, periodic %llu then %llu; "
           "CPU 0's timer %llu\n",
           (unsigned long long)lowered, (unsigned long long)t1, (unsigned long long)timed_out,
           (unsigned long long)p1, (unsigned long long)p2, (unsigned long long)t0);
    int near = 40;
#define NEAR(t) ((t) + near >= lowered && (t) <= lowered + near)
    check("a timer armed there fires when the priority drops", lowered >= 350 && NEAR(t1));
    check("a timeout armed there ends when the priority drops", NEAR(timed_out));
    check("a periodic timer fires once, then a period later", NEAR(p1) && p2 >= p1 + 5 && p2 <= p1 + 20);
    check("... with no overruns", ti.overruns == 0);
    check("a timer armed on CPU 0 fires on time", t0 >= 100 && t0 < 200);
    timer_delete(on1);
    timer_delete(on0);
    timer_delete(every);
}

int main(void)
{
    cps = SYSPAGE_ENTRY(qtime)->cycles_per_sec;
    if (_syspage_ptr->num_cpu < 2) {
        printf("offlinetest: needs 2 CPUs\n");
        return 1;
    }
    struct _sched_info si;
    SchedInfo(0, SCHED_OFFLINING, &si);
    check("SCHED_OFFLINING goes up to 254", si.priority_max == 254);
    SchedInfo(0, SCHED_FIFO, &si);
    check("SCHED_FIFO up to 253", si.priority_max == 253);

    /* SchedSet() */
    check("SCHED_FIFO 254: EINVAL", setp(SCHED_FIFO, 254) == EINVAL);
    check("SCHED_HIGH_PRIORITY_IST, not an interrupt thread: EINVAL",
          setp(SCHED_HIGH_PRIORITY_IST, 100) == EINVAL);
    check("SCHED_OFFLINING 255: EINVAL", setp(SCHED_OFFLINING, 255) == EINVAL);
    pthread_t other;
    pthread_create(&other, NULL, idle_thread, NULL);
    struct sched_param sp = {.sched_priority = 10};
    check("SCHED_OFFLINING for another thread: EINVAL",
          SchedSet_r(0, other, SCHED_OFFLINING, &sp) == EINVAL);
    check("tid -1: ENOSYS", SchedSet_r(0, -1, SCHED_FIFO, &sp) == ENOSYS);
    pid_t pid = fork();
    if (pid == 0) {
        setuid(100);
        _exit(setp(SCHED_OFFLINING, 10));
    }
    int st;
    waitpid(pid, &st, 0);
    check("SCHED_OFFLINING without the runstate ability: EPERM", WIFEXITED(st) && WEXITSTATUS(st) == EPERM);

    /* As a SCHED_FIFO thread. */
    check("SchedCtl(OFFLINE), SCHED_FIFO: EPERM", SchedCtl_r(SCHED_PROCESSOR_OFFLINE, NULL, 0) == EPERM);
    check("TimerDelegate(9): ENOSYS", TimerDelegate_r(9, -1, 0) == -ENOSYS);
    check("TimerDelegate(QUERY) of CPU 9: EINVAL",
          TimerDelegate_r(_NTO_TIMER_QUERY_DELEGATE, 9, -1) == -EINVAL);
    check("TimerDelegate(DELEGATE) to CPU 9: EINVAL", TimerDelegate_r(_NTO_TIMER_DELEGATE, -1, 9) == -EINVAL);
    check("TimerDelegate(QUERY), SCHED_FIFO: EPERM",
          TimerDelegate_r(_NTO_TIMER_QUERY_DELEGATE, -1, -1) == -EPERM);

    check("SCHED_OFFLINING 10", setp(SCHED_OFFLINING, 10) == EOK);
    check("SchedGet(): SCHED_OFFLINING", SchedGet(0, 0, NULL) == SCHED_OFFLINING);
    check("TimerDelegate(), not pinned: ENOEXEC",
          TimerDelegate_r(_NTO_TIMER_QUERY_DELEGATE, -1, -1) == -ENOEXEC);
    check("SchedCtl(OFFLINE), not pinned: EINVAL", SchedCtl_r(SCHED_PROCESSOR_OFFLINE, NULL, 0) == EINVAL);
    pin(1);
    check("TimerDelegate() for another CPU: ENOSYS", TimerDelegate_r(_NTO_TIMER_QUERY_DELEGATE, 0, -1) == -ENOSYS);
    check("TimerDelegate(DELEGATE) to itself: EINVAL", TimerDelegate_r(_NTO_TIMER_DELEGATE, -1, 1) == -EINVAL);
    check("TimerDelegate(), priority 10: ERANGE", TimerDelegate_r(_NTO_TIMER_DELEGATE, -1, 0) == -ERANGE);
    setp(SCHED_OFFLINING, 254);
    check("TimerDelegate(), the clock's priority: ERANGE",
          TimerDelegate_r(_NTO_TIMER_RECLAIM, -1, -1) == -ERANGE);
    check("SchedCtl(ONLINE), online: EAGAIN", SchedCtl_r(SCHED_PROCESSOR_ONLINE, NULL, 0) == EAGAIN);
    setp(SCHED_FIFO, 10);
    pin(0);

    /* CPU 1 offline: its FIFO 20 thread waits. */
    pthread_t c, o;
    pthread_create(&c, NULL, counter_thread, NULL);
    usleep(100000);
    pthread_create(&o, NULL, offliner, NULL);
    pthread_join(o, NULL);
    check("SchedCtl(OFFLINE)", r_off == EOK);
    check("SchedCtl(OFFLINE) again: EAGAIN", r_off2 == EAGAIN);
    check("SCHED_PROCESSORS_STATUS leaves CPU 1 out",
          status_online == _syspage_ptr->num_cpu - 1 && !(status_mask & 2) && (status_mask & 1));
    check("offline, CPU 1 ran no other thread", during == 0);
    check("SchedCtl(ONLINE)", r_on == EOK);
    check("SchedCtl(ONLINE) again: EAGAIN", r_on2 == EAGAIN);
    long after = counter;
    usleep(100000);
    check("online, the thread runs again", counter > after);
    stop = 1;
    pthread_join(c, NULL);

    /* A high-resolution timer armed on CPU 1 waits for the thread at the
     * clock's priority there to drop its priority. */
    int chid = ChannelCreate(0);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, coid, 10, 1, 0);
    timer_t tid;
    pin(1);
    timer_create(CLOCK_MONOTONIC, &ev, &tid);
    struct itimerspec tol = {{0, 1000}, {0, 0}};
    timer_settime(tid, TIMER_TOLERANCE, &tol, NULL);
    struct itimerspec it = {{0, 100000000}, {0, 0}};
    uint64_t c0 = ClockCycles();
    timer_settime(tid, 0, &it, NULL);
    pin(0);
    pthread_t h;
    pthread_create(&h, NULL, holder, &c0);
    struct _pulse pl;
    MsgReceivePulse(chid, &pl, sizeof pl, NULL);
    uint64_t got = ms_since(c0);
    pthread_join(h, NULL);
    check("a timer due at 100 ms fires when the priority drops (400 ms)",
          got >= 350 && got + 50 >= lowered && got <= lowered + 50);
    if (got < 350 || got > lowered + 50) {
        printf("offlinetest:   fired at %llu ms, lowered at %llu ms\n", (unsigned long long)got,
               (unsigned long long)lowered);
    }

    held_timers(chid, coid);

    printf("offlinetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

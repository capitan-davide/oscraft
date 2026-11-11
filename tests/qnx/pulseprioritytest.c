/* The priority a pulse gives the thread that receives it (review item
 * SCHED-22), as procnto (measured): the pulse's priority, capped at the
 * highest the receiving process may take, whoever sent it: 253 for root
 * (sched_get_priority_max()), 63 for an unprivileged process. Pulses from
 * MsgSendPulse() and from a timer's SIGEV_PULSE; root's pulses to an
 * unprivileged process. The kernel used to pass any priority up to 255
 * through, so an unprivileged server could be run above the clock's
 * interrupt thread. Prints one "ok"/"FAIL" line per check and
 * "pulseprioritytest: passed". */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *who, int sent, int got, int want)
{
    printf("pulseprioritytest: %s %s, a pulse at %d runs it at %d (want %d)\n", got == want ? "ok" : "FAIL", who,
           sent, got, want);
    fflush(stdout);
    fails += got != want;
}

static int cur(void)
{
    struct sched_param p;
    int pol;
    pthread_getschedparam(pthread_self(), &pol, &p);
    return p.sched_curpriority;
}

static void reset(void)
{
    struct sched_param sp = { .sched_priority = 10 };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
}

static const int prios[] = { 63, 64, 200, 253, 254, 255 };
#define N (int)(sizeof prios / sizeof *prios)

static int capped(int prio, int cap)
{
    return prio < cap ? prio : cap;
}

/* the process's own pulses, and a timer's at 254 */
static void own(const char *who, int cap)
{
    int chid = ChannelCreate(0);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    struct _pulse pu;
    for (int i = 0; i < N; i++) {
        MsgSendPulse(coid, prios[i], 1, 0);
        MsgReceive(chid, &pu, sizeof pu, NULL);
        check(who, prios[i], cur(), capped(prios[i], cap));
        reset();
    }
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, coid, 254, 2, 0);
    timer_t t;
    if (timer_create(CLOCK_MONOTONIC, &ev, &t) == 0) {
        struct itimerspec it = { .it_value = { 0, 1000000 } };
        timer_settime(t, 0, &it, NULL);
        MsgReceive(chid, &pu, sizeof pu, NULL);
        char what[64];
        snprintf(what, sizeof what, "%s (a timer)", who);
        check(what, 254, cur(), capped(254, cap));
        reset();
        timer_delete(t);
    }
    ConnectDetach(coid);
    ChannelDestroy(chid);
}

static void as_user(void)
{
    setgid(100);
    setuid(100);
    reset();
}

int main(void)
{
    reset();
    own("root", 253);

    pid_t c = fork();
    if (c == 0) {
        as_user();
        own("an unprivileged process", 63);
        _exit(fails != 0);
    }
    int st;
    waitpid(c, &st, 0);
    fails += !WIFEXITED(st) || WEXITSTATUS(st) != 0;

    /* root's pulses to an unprivileged process: each received in turn */
    int fd[2], back[2];
    pipe(fd);
    pipe(back);
    c = fork();
    if (c == 0) {
        as_user();
        int ch = ChannelCreate(0);
        write(fd[1], &ch, sizeof ch);
        for (int i = 0; i < N; i++) {
            struct _pulse pu;
            MsgReceive(ch, &pu, sizeof pu, NULL);
            check("an unprivileged process from root", pu.value.sival_int, cur(), capped(pu.value.sival_int, 63));
            reset();
            write(back[1], &i, sizeof i);
        }
        _exit(fails != 0);
    }
    int ch;
    read(fd[0], &ch, sizeof ch);
    int coid = ConnectAttach(0, c, ch, _NTO_SIDE_CHANNEL, 0);
    for (int i = 0; i < N; i++) {
        int k;
        MsgSendPulse(coid, prios[i], 1, prios[i]);
        read(back[0], &k, sizeof k);
    }
    waitpid(c, &st, 0);
    fails += !WIFEXITED(st) || WEXITSTATUS(st) != 0;
    printf("pulseprioritytest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

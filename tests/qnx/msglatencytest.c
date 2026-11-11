/* A long message must not keep a higher-priority thread off its CPU
 * (review items IPC-10, CORE-6). A SCHED_FIFO thread at 60 wakes every
 * 1 ms (absolute deadlines) on CPU 1, where a thread at 10 sends 64 MiB
 * messages to a server on CPU 2 for 2 s. On procnto the wakeups are at
 * most 0.8 ms late (measured, QEMU); OsCraft copied each message as one
 * piece with interrupts masked, and they were up to 176 ms late. The
 * check allows 20 ms, for QEMU's noise. Prints the lateness and
 * "msglatencytest: passed". */
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <time.h>
#include <unistd.h>

#define SECS 2
#define MSG (64 << 20)
#define LIMIT_US 20000

static volatile int stop;
static int chid;

static long long ns(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000LL + t.tv_nsec;
}

static void pin(int cpu)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
}

static void prio(int p)
{
    struct sched_param sp = { .sched_priority = p };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
}

static void *server(void *a)
{
    (void)a;
    pin(2);
    prio(10);
    char *buf = malloc(MSG);
    for (;;) {
        int rcvid = MsgReceive(chid, buf, MSG, NULL);
        if (rcvid > 0) {
            MsgReply(rcvid, 0, NULL, 0);
        }
    }
    return NULL;
}

static long sends;

static void *sender(void *a)
{
    (void)a;
    pin(1);
    prio(10);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    char *msg = malloc(MSG);
    memset(msg, 1, MSG);
    while (!stop) {
        if (MsgSend(coid, msg, MSG, NULL, 0) == 0) {
            sends++;
        }
    }
    return NULL;
}

int main(void)
{
    chid = ChannelCreate(0);
    pthread_t s, l;
    pthread_create(&s, NULL, server, NULL);
    pin(1);
    prio(60);
    pthread_create(&l, NULL, sender, NULL);
    usleep(100000);
    long long worst = 0;
    struct timespec next;
    clock_gettime(CLOCK_MONOTONIC, &next);
    for (int i = 0; i < SECS * 1000; i++) {
        next.tv_nsec += 1000000;
        if (next.tv_nsec >= 1000000000) {
            next.tv_nsec -= 1000000000;
            next.tv_sec++;
        }
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL);
        long long late = ns() - (next.tv_sec * 1000000000LL + next.tv_nsec);
        if (late > worst) {
            worst = late;
        }
    }
    stop = 1;
    pthread_join(l, NULL);
    int ok = sends > 0 && worst / 1000 < LIMIT_US;
    printf("msglatencytest: %ld messages of 64 MiB; worst wakeup %lld us late\n", sends, worst / 1000);
    printf("msglatencytest: %s a 1 ms thread at 60 stays within %d us of its deadlines\n", ok ? "ok" : "FAIL",
           LIMIT_US);
    printf("msglatencytest: %s\n", ok ? "passed" : "FAILED");
    return !ok;
}

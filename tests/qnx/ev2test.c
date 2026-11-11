/* More events (as procnto):
 * - SIGEV_MEMORY from a timer, each sigev_memop, and an invalid one;
 * - SIGEV_SEM from a timer, on a named and an anonymous semaphore;
 * - SIGEV_PULSE with a negative sigev_coid (a channel of the receiver's);
 * - TimerTimeout() with an event other than SIGEV_UNBLOCK: EINVAL;
 * - timer IDs: per process, the lowest free one from 0; TimerInfo() with
 *   _NTO_TIMER_SEARCH.
 * Prints one "ok"/"FAIL" line per check and "ev2test: passed". */
#include <errno.h>
#include <fcntl.h>
#include <semaphore.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <stdint.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("ev2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* A one-shot timer in 10 ms delivering `ev`; timer_create()'s errno, or 0. */
static int fire(struct sigevent *ev)
{
    timer_t t;
    if (timer_create(CLOCK_MONOTONIC, ev, &t) != 0) {
        return errno;
    }
    struct itimerspec its = { .it_value = { 0, 10000000 } };
    timer_settime(t, 0, &its, NULL);
    usleep(50000);
    timer_delete(t);
    return 0;
}

static volatile unsigned word;

static unsigned memop(unsigned start, int op, int value)
{
    struct sigevent ev;
    word = start;
    SIGEV_MEMORY_INIT(&ev, &word, value, op);
    int r = fire(&ev);
    printf("ev2test:   memop %d: %#x %d -> %#x (%d)\n", op, start, value, word, r);
    return word;
}


int main(void)
{
    struct sigevent ev;

    check("SIGEV_MEM_ASSIGN", memop(1, SIGEV_MEM_ASSIGN, 5) == 5);
    check("SIGEV_MEM_ADD", memop(5, SIGEV_MEM_ADD, 3) == 8);
    check("SIGEV_MEM_SUB", memop(8, SIGEV_MEM_SUB, 1) == 7);
    check("SIGEV_MEM_BITSET", memop(7, SIGEV_MEM_BITSET, 0x30) == 0x37);
    check("SIGEV_MEM_BITCLR", memop(0x37, SIGEV_MEM_BITCLR, 3) == 0x34);
    check("SIGEV_MEM_BITTOGGLE", memop(0x34, SIGEV_MEM_BITTOGGLE, 5) == 0x31);
    word = 9;
    SIGEV_MEMORY_INIT(&ev, &word, 1, 7);
    int r = fire(&ev);
    printf("ev2test:   invalid memop: timer_create %d, word %u\n", r, word);
    check("an invalid sigev_memop from a timer: accepted, nothing done", r == 0 && word == 9);
    SIGEV_MEMORY_INIT(&ev, &word, 4, SIGEV_MEM_ASSIGN);
    r = MsgDeliverEvent(0, &ev);
    check("MsgDeliverEvent() of SIGEV_MEMORY to itself", r == 0 && word == 4);
    SIGEV_MEMORY_INIT(&ev, &word, 4, 0);
    r = MsgDeliverEvent(0, &ev);
    printf("ev2test:   MsgDeliverEvent() with memop 0: %d %d\n", r, r < 0 ? errno : 0);
    check("MsgDeliverEvent() of an invalid sigev_memop: EINVAL", r == -1 && errno == EINVAL);
    SIGEV_MEMORY_INIT(&ev, (volatile unsigned *)(uintptr_t)main, 4, SIGEV_MEM_ASSIGN);
    r = fire(&ev);
    check("SIGEV_MEMORY on read-only memory fails quietly", r == 0);

    sem_t *s = sem_open("/ev2test", O_CREAT, 0644, 0);
    check("sem_open()", s != SEM_FAILED);
    SIGEV_SEM_INIT(&ev, s);
    r = fire(&ev);
    int v = -1;
    sem_getvalue(s, &v);
    printf("ev2test:   SIGEV_SEM: timer_create %d, value %d\n", r, v);
    check("SIGEV_SEM posts a named semaphore", r == 0 && v == 1 && sem_trywait(s) == 0);
    sem_close(s);
    sem_unlink("/ev2test");
    s = sem_open(SEM_ANON, O_CREAT, 0600, 0);
    SIGEV_SEM_INIT(&ev, s);
    r = fire(&ev);
    check("SIGEV_SEM posts an anonymous named semaphore", r == 0 && sem_trywait(s) == 0);
    sem_close(s);

    int chid = ChannelCreate(_NTO_CHF_PRIVATE);
    SIGEV_PULSE_INIT(&ev, -chid, SIGEV_PULSE_PRIO_INHERIT, 7, 1234);
    r = fire(&ev);
    struct _pulse p;
    memset(&p, 0, sizeof p);
    struct _msg_info info;
    uint64_t ns = 100000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
    int rcvid = MsgReceive(chid, &p, sizeof p, &info);
    printf("ev2test:   negative coid: timer_create %d, rcvid %d, code %d value %d scoid %d\n", r, rcvid, p.code,
           p.value.sival_int, p.scoid);
    check("SIGEV_PULSE on a channel of its own (negative sigev_coid)",
          r == 0 && rcvid == 0 && p.code == 7 && p.value.sival_int == 1234);

    /* TimerTimeout() takes only SIGEV_UNBLOCK (or NULL). */
    SIGEV_PULSE_INIT(&ev, -chid, SIGEV_PULSE_PRIO_INHERIT, 8, 99);
    ns = 20000000;
    r = TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, &ev, &ns, NULL);
    check("TimerTimeout() with a pulse: EINVAL", r == -1 && errno == EINVAL);

    struct sigevent none;
    SIGEV_NONE_INIT(&none);
    timer_t ids[4];
    for (int i = 0; i < 3; i++) {
        timer_create(CLOCK_MONOTONIC, &none, &ids[i]);
    }
    printf("ev2test:   timer ids %d %d %d\n", (int)ids[0], (int)ids[1], (int)ids[2]);
    check("timer IDs from 0", ids[0] == 0 && ids[1] == 1 && ids[2] == 2);
    timer_delete(ids[1]);
    timer_create(CLOCK_REALTIME, &none, &ids[3]);
    check("a new timer takes the lowest free ID", ids[3] == 1);
    struct _timer_info ti;
    check("TimerInfo(_NTO_TIMER_SEARCH) from 2 finds 2", TimerInfo(0, 2, _NTO_TIMER_SEARCH, &ti) == 2);
    check("TimerInfo(_NTO_TIMER_SEARCH) past the last: EINVAL",
          TimerInfo(0, 3, _NTO_TIMER_SEARCH, &ti) == -1 && errno == EINVAL);

    printf("ev2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

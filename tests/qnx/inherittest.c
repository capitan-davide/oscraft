/* A server's own priority through message-driven priority inheritance
 * (review item SCHED-34), as procnto (measured): a receive makes the
 * client's (or the pulse's) priority the server thread's own, its base
 * priority, which pthread_getschedparam() reports and which stays after
 * the reply until the next receive. A server boosted while busy only
 * runs higher (its base stays). A mutex waiter's raise stays above a
 * received priority, and the server falls back to the received one when
 * it lets the mutex go. The kernel changed only the current priority, so
 * the base stayed the server's, and a receive overwrote a mutex raise
 * (30 down to 20 while the waiter still waited). Prints one "ok"/"FAIL"
 * line per check and "inherittest: passed". */
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <unistd.h>

static int fails, chid;
static uint64_t cps;
static pthread_mutex_t mx;

static void check(const char *when, int base, int current)
{
    struct sched_param p;
    int pol;
    pthread_getschedparam(pthread_self(), &pol, &p);
    int ok = p.sched_priority == base && p.sched_curpriority == current;
    printf("inherittest: %s %s: base %d current %d (want %d %d)\n", ok ? "ok" : "FAIL", when, p.sched_priority,
           p.sched_curpriority, base, current);
    fflush(stdout);
    fails += !ok;
}

static void setp(int prio)
{
    struct sched_param sp = { .sched_priority = prio };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
}

static void spin_ms(int n)
{
    uint64_t end = ClockCycles() + cps * n / 1000;
    while (ClockCycles() < end) {
    }
}

static void *client(void *a)
{
    setp((int)(intptr_t)a);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    char m = 1, r;
    MsgSend(coid, &m, 1, &r, 1);
    return NULL;
}

static void *waiter(void *a)
{
    (void)a;
    setp(30);
    pthread_mutex_lock(&mx);
    pthread_mutex_unlock(&mx);
    return NULL;
}

/* receive one message from a client at `prio` and reply */
static void serve(int prio, const char *received, int base, int current)
{
    pthread_t c;
    pthread_create(&c, NULL, client, (void *)(intptr_t)prio);
    char m;
    int rcvid = MsgReceive(chid, &m, 1, NULL);
    check(received, base, current);
    MsgReply(rcvid, 0, &m, 1);
    pthread_join(c, NULL);
}

int main(void)
{
    cps = SYSPAGE_ENTRY(qtime)->cycles_per_sec;
    setp(10);
    chid = ChannelCreate(0);
    check("start", 10, 10);
    serve(20, "received from a client at 20", 20, 20);
    check("after the reply", 20, 20);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    MsgSendPulse(coid, 5, 1, 0);
    struct _pulse pu;
    MsgReceive(chid, &pu, sizeof pu, NULL);
    check("received a pulse at 5", 5, 5);
    serve(15, "received from a client at 15", 15, 15);

    /* boosted while busy */
    setp(10);
    pthread_t c;
    pthread_create(&c, NULL, client, (void *)20);
    spin_ms(50);
    check("busy, a client at 20 waiting (server boost)", 10, 20);
    char m;
    int rcvid = MsgReceive(chid, &m, 1, NULL);
    check("then received it", 20, 20);
    MsgReply(rcvid, 0, &m, 1);
    pthread_join(c, NULL);

    /* a mutex waiter at 30, then a receive at 20 */
    setp(10);
    pthread_mutex_init(&mx, NULL);
    pthread_mutex_lock(&mx);
    pthread_t w;
    pthread_create(&w, NULL, waiter, NULL);
    usleep(20000);
    check("holding a mutex a thread at 30 waits for", 10, 30);
    serve(20, "and received from a client at 20", 20, 30);
    pthread_mutex_unlock(&mx);
    pthread_join(w, NULL);
    check("the mutex let go", 20, 20);
    printf("inherittest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

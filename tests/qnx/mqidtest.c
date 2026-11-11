/* Kernel message queues as a queue manager sees them (as procnto):
 * - a queue's ID is the lowest free slot of the manager's channel-ID
 *   space: a channel made later skips it, and MsgQueueClose(-1) frees it;
 * - a binding is the server connection's: a client process's connections
 *   to the manager's channel share one (a second MsgQueueOpen() for any
 *   of them, or from another of its threads, is EBUSY, and a send on
 *   another of them reaches the queue), but _NTO_COF_NOSHARE ones;
 * - IDs name only the manager's own queues: ESRCH for a closed one, a
 *   channel's or another manager's; MsgQueueClose(-1) of a channel's ID
 *   is EINVAL.
 * Prints one "ok"/"FAIL" line per check and "mqidtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/iofunc.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static int chid;
static pid_t mgr;

static void check(const char *what, int ok)
{
    printf("mqidtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int csend(int coid)
{
    char c = 0;
    return MsgSend(coid, &c, 1, NULL, 0);
}

/* A client process: `n` messages, each on a new connection if `newconn`
 * (2: made with _NTO_COF_NOSHARE), then it waits to be killed. */
static pid_t clientproc(int n, int newconn)
{
    pid_t p = fork();
    if (p == 0) {
        int coid = -1;
        for (int i = 0; i < n; i++) {
            if (coid == -1 || newconn)
                coid = ConnectAttach(0, mgr, chid, _NTO_SIDE_CHANNEL, newconn == 2 ? _NTO_COF_NOSHARE : 0);
            csend(coid);
        }
        pause();
        _exit(0);
    }
    return p;
}

static void *tsend(void *a)
{
    csend(ConnectAttach(0, mgr, chid, _NTO_SIDE_CHANNEL, 0));
    return a;
}

/* A client process: a message from each of two threads in turn, each on
 * its own connection. */
static pid_t twothreads(void)
{
    pid_t p = fork();
    if (p == 0) {
        for (int i = 0; i < 2; i++) {
            pthread_t t;
            pthread_create(&t, NULL, tsend, NULL);
            pthread_join(t, NULL);
        }
        pause();
        _exit(0);
    }
    return p;
}

/* Receive the next client message and MsgQueueOpen() for it: the result,
 * or -errno. */
static int open1(int id)
{
    char c;
    rcvid_t rcvid = MsgReceive(chid, &c, 1, NULL);
    int r = MsgQueueOpen(rcvid, id, 16, 4, _IO_FLAG_RD | _IO_FLAG_WR);
    int e = errno;
    MsgReply(rcvid, 0, NULL, 0);
    return r == -1 ? -e : r;
}

static int err(int r)
{
    return r == -1 ? errno : 0;
}

/* Our own two plain connections: a queue bound through the first, a
 * message sent on the second, received on the first. */
static int shared_coid[2];
static void *tsend2(void *a)
{
    csend(shared_coid[0]);
    return a;
}
static int shared_send(void)
{
    shared_coid[0] = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    shared_coid[1] = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    pthread_t t;
    pthread_create(&t, NULL, tsend2, NULL);
    int q = open1(-1);
    pthread_join(t, NULL);
    int s = MsgQueueSend(shared_coid[1], "hi", 3, 1);
    char buf[16];
    unsigned prio = 0;
    int r = MsgQueueReceive(shared_coid[0], buf, sizeof buf, &prio);
    printf("mqidtest:   own connections: queue %d, send on the other %d, receive %d\n", q, s, r);
    MsgQueueClose(-1, q);
    return q > 0 && s == 0 && r == 3 && strcmp(buf, "hi") == 0;
}

int main(void)
{
    mgr = getpid();
    chid = ChannelCreate(0);
    check("our channel is 1", chid == 1);
    pid_t a = clientproc(1, 0), b = clientproc(1, 0);
    int qa = open1(-1), qb = open1(-1);
    printf("mqidtest:   two client processes: queues %d %d\n", qa, qb);
    check("queue IDs follow the channel's: 2, 3", qa == 2 && qb == 3);
    pid_t n = clientproc(2, 2);
    int n1 = open1(-1), n2 = open1(-1);
    check("_NTO_COF_NOSHARE connections of one process: a queue each", n1 == 4 && n2 == 5);
    pid_t c = clientproc(2, 1);
    int c1 = open1(-1), c2 = open1(-1);
    printf("mqidtest:   two connections of one process: %d %d\n", c1, c2);
    check("two plain connections of one process: one binding, EBUSY", c1 == 6 && c2 == -EBUSY);
    pid_t t = twothreads();
    int t1 = open1(-1), t2 = open1(-1);
    check("another thread of the process: EBUSY", t1 == 7 && t2 == -EBUSY);
    pid_t d = clientproc(2, 1);
    int d1 = open1(qa), d2 = open1(qa);
    check("an existing queue twice from one process: 0, EBUSY", d1 == 0 && d2 == -EBUSY);
    int ch = ChannelCreate(0);
    check("a new channel skips the queues' IDs: 8", ch == 8);
    check("MsgQueueClose(-1) of a channel's ID: EINVAL", err(MsgQueueClose(-1, ch)) == EINVAL);
    check("MsgQueueClose(-1, queue 2)", MsgQueueClose(-1, qa) == 0);
    check("MsgQueueClose(-1) again: EINVAL", err(MsgQueueClose(-1, qa)) == EINVAL);
    int ch2 = ChannelCreate(0);
    check("its ID is free: the next channel is 2", ch2 == 2);
    pid_t e = clientproc(2, 2);
    int e1 = open1(qa), e2 = open1(ch);
    check("opening a channel's ID: ESRCH", e1 == -ESRCH && e2 == -ESRCH);
    check("a send on another connection that shares the binding", shared_send());

    pid_t other = fork();
    if (other == 0) {
        mgr = getpid();
        chid = ChannelCreate(0);
        pid_t f = clientproc(1, 0);
        int r = open1(qb);
        kill(f, SIGKILL);
        _exit(r == -ESRCH ? 0 : 1);
    }
    int st;
    waitpid(other, &st, 0);
    check("another manager opening our queue's ID: ESRCH", WIFEXITED(st) && WEXITSTATUS(st) == 0);

    pid_t kids[] = { a, b, n, c, t, d, e };
    for (unsigned i = 0; i < sizeof kids / sizeof kids[0]; i++)
        kill(kids[i], SIGKILL);
    while (wait(NULL) > 0) {
    }
    printf("mqidtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

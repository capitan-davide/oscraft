/* A server replying from a bad buffer of its own (review item IPC-8), as
 * procnto does (measured): MsgReply() fails with ESRVRFAULT, the client's
 * MsgSend() fails with ESRVRFAULT too, and the message is over (a second
 * reply is ESRCH). So for MsgReplyv() with an unreadable iov array. A
 * reply of 0 bytes from a bad address works. MsgWrite() from a bad
 * buffer fails with ESRVRFAULT and leaves the client waiting. The kernel
 * used to charge the server's fault to the client (EFAULT) and tell the
 * server its reply had worked, and to leave the client blocked after an
 * unreadable iov array. Prints one "ok"/"FAIL" line per check and
 * "replyfaulttest: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;
static int chid, coid;
static volatile long cr;
static volatile int cerr;
static char ans[64];

static void check(const char *what, int ok)
{
    printf("replyfaulttest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *client(void *a)
{
    (void)a;
    errno = 0;
    cr = MsgSend(coid, "hello", 6, ans, sizeof ans);
    cerr = errno;
    return NULL;
}

/* a client sends; returns the rcvid of its message */
static int start(pthread_t *t)
{
    cr = -2;
    pthread_create(t, NULL, client, NULL);
    char m[16];
    return MsgReceive(chid, m, sizeof m, NULL);
}

static void one(const char *what, void *buf, long len)
{
    char line[160];
    pthread_t t;
    int rcvid = start(&t);
    errno = 0;
    int r = MsgReply(rcvid, 5, buf, len);
    int e = errno;
    pthread_join(t, NULL);
    snprintf(line, sizeof line, "%s: MsgReply -> %d errno %d, client %ld errno %d", what, r, e, cr, cerr);
    check(line, r == -1 && e == ESRVRFAULT && cr == -1 && cerr == ESRVRFAULT);
    errno = 0;
    r = MsgReply(rcvid, 9, "ok", 3);
    snprintf(line, sizeof line, "%s: the message is over (a second reply: ESRCH)", what);
    check(line, r == -1 && errno == ESRCH);
}

int main(void)
{
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    one("from unmapped memory", (void *)0x1000, 16);
    one("from a wrapping buffer", (void *)(UINT64_MAX - 7), 16);
    one("from past the end of the address space", (void *)((1ULL << 39) - 8), 16);

    pthread_t t;
    int rcvid = start(&t);
    int r = MsgReply(rcvid, 5, (void *)0x1000, 0);
    pthread_join(t, NULL);
    check("0 bytes from unmapped memory: the reply works, the client gets its status", r == 0 && cr == 5);

    rcvid = start(&t);
    errno = 0;
    r = MsgReplyv(rcvid, 5, (iov_t *)0x1000, 2);
    int e = errno;
    pthread_join(t, NULL);
    char line[160];
    snprintf(line, sizeof line, "MsgReplyv with an unreadable iov array -> %d errno %d, client %ld errno %d", r, e, cr, cerr);
    check(line, r == -1 && e == ESRVRFAULT && cr == -1 && cerr == ESRVRFAULT);

    rcvid = start(&t);
    errno = 0;
    long w = MsgWrite(rcvid, (void *)0x1000, 16, 0);
    e = errno;
    usleep(20000);
    snprintf(line, sizeof line, "MsgWrite from unmapped memory -> %ld errno %d, the client still waits", w, e);
    check(line, w == -1 && e == ESRVRFAULT && cr == -2);
    MsgReply(rcvid, 7, "ok", 3);
    pthread_join(t, NULL);
    check("then the reply reaches it", cr == 7);
    printf("replyfaulttest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

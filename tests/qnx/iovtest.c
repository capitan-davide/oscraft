/* Message buffers (iovs) with lengths near the top of the range, past the
 * end of the address space or in unmapped memory (review IPC-4): what
 * each side's call returns, as procnto does (measured):
 * - a client's send buffers must lie within the address space (their
 *   ends computed without wrapping) and each start in readable memory,
 *   whether or not the server takes the bytes: EFAULT at MsgSendv(),
 *   the server untouched; a buffer running on into unmapped memory the
 *   server doesn't reach is fine;
 * - a server's receive buffers must lie within the address space, used
 *   or not: EFAULT at MsgReceivev() and ESRVRFAULT for the client whose
 *   message it was, and a pulse is lost with the receive; receive
 *   buffers in unmapped memory the message reaches are EFAULT and
 *   ESRVRFAULT too, ones it doesn't reach are fine;
 * - a client's reply buffers aren't checked when it sends (srcmsglen and
 *   dstmsglen in struct _msg_info are the sums, wrapped around); when a
 *   reply with any bytes comes, the first of them must lie within the
 *   address space (EFAULT for the client, the reply succeeds), the
 *   others only where the reply reaches them;
 * - MsgReadv() and MsgWritev() with offsets past the message return 0;
 *   MsgReadv() into the server's own buffers past the address space or
 *   in unmapped memory is ESRVRFAULT, MsgWritev() from unmapped memory
 *   too (from a wrapping buffer it writes what the client takes);
 * - /proc/<pid>/as read or written at an unmapped or wrapping range is
 *   ESRVRFAULT.
 * Each corner runs with a server thread of its own, which receives with
 * the corner's buffers and replies. Prints one "ok"/"FAIL" line per
 * check and "iovtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <unistd.h>

#define UEND (1ULL << 39)
#define NOCHECK INT64_MIN

static int fails;
static int chid;
static int coid;
static char sbuf[64] = "hello, world: a message of 64 bytes for the iov test..........";
static char rbuf[256];
static char rep[64] = "reply bytes, 16";
static char ans[64];

static void check(const char *what, int ok)
{
    printf("iovtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* what a corner expects */
struct expect {
    long client_r;      /* MsgSendv()'s return, -1 for an error */
    int client_errno;
    int received;       /* 1: the server received (rcvid > 0); 0: its receive failed; -1: it never got the message */
    int server_errno;   /* when received == 0 */
    int msglen;         /* when received == 1 */
    long long srcmsglen, dstmsglen;  /* when received == 1, or NOCHECK */
};

/* the server thread of a corner */
struct server {
    iov_t riov[2];
    int rparts;
    int reply_len;
    int rcvid, err, msglen;
    long long srcmsglen, dstmsglen;
    int reply_r, reply_errno;
};

static void *server_thread(void *arg)
{
    struct server *s = arg;
    struct _msg_info info;
    memset(&info, 0, sizeof info);
    memset(rbuf, 0, sizeof rbuf);
    errno = 0;
    s->rcvid = MsgReceivev(chid, s->riov, s->rparts, &info);
    s->err = errno;
    if (s->rcvid > 0) {
        s->msglen = info.msglen;
        s->srcmsglen = info.srcmsglen;
        s->dstmsglen = info.dstmsglen;
        errno = 0;
        s->reply_r = MsgReply(s->rcvid, 5, rep, s->reply_len);
        s->reply_errno = errno;
    }
    return NULL;
}

static void corner(const char *name, iov_t *siov, int sparts, iov_t *criov, int crparts, struct server *s,
                   const struct expect *e)
{
    char what[160];
    pthread_t t;
    s->rcvid = -2;
    pthread_create(&t, NULL, server_thread, s);
    usleep(20000);
    errno = 0;
    long r = MsgSendv(coid, siov, sparts, criov, crparts);
    int err = errno;
    usleep(20000);
    if (s->rcvid == -2) {
        /* the server never got the message: a pulse ends its receive */
        MsgSendPulse(coid, 10, 1, 0);
    }
    pthread_join(t, NULL);
    snprintf(what, sizeof what, "%s: client %ld errno %d", name, r, err);
    check(what, r == e->client_r && (r != -1 || err == e->client_errno));
    if (e->received == 1) {
        snprintf(what, sizeof what, "%s: server received %d bytes (rcvid %d errno %d), srcmsglen %lld, dstmsglen %lld",
                 name, s->msglen, s->rcvid, s->err, s->srcmsglen, s->dstmsglen);
        check(what, s->rcvid > 0 && s->msglen == e->msglen
                        && (e->srcmsglen == NOCHECK || s->srcmsglen == e->srcmsglen)
                        && (e->dstmsglen == NOCHECK || s->dstmsglen == e->dstmsglen));
        snprintf(what, sizeof what, "%s: server's reply %d errno %d", name, s->reply_r, s->reply_errno);
        check(what, s->reply_r == 0);
    } else if (e->received == 0) {
        snprintf(what, sizeof what, "%s: server's receive failed, rcvid %d errno %d", name, s->rcvid, s->err);
        check(what, s->rcvid == -1 && s->err == e->server_errno);
    } else {
        snprintf(what, sizeof what, "%s: server never got the message (rcvid %d)", name, s->rcvid);
        check(what, s->rcvid == 0);
    }
}

static struct server srv(uint64_t len0, void *base1, uint64_t len1, int reply_len)
{
    struct server s;
    memset(&s, 0, sizeof s);
    s.riov[0].iov_base = rbuf;
    s.riov[0].iov_len = len0;
    s.riov[1].iov_base = base1;
    s.riov[1].iov_len = len1;
    s.rparts = base1 ? 2 : 1;
    s.reply_len = reply_len;
    return s;
}

int main(void)
{
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    iov_t ok_s[1] = { { sbuf, 64 } };
    iov_t ok_r[1] = { { ans, 64 } };
    struct server s;
    const struct expect send_fault = { -1, EFAULT, -1, 0, 0, 0, 0 };
    const struct expect served = { 5, 0, 1, 0, 64, 64, 64 };

    /* the client's send buffers */
    { iov_t v[1] = { { sbuf, UINT64_MAX } }; s = srv(256, NULL, 0, 16);
      corner("send iov wraps", v, 1, ok_r, 1, &s, &send_fault); }
    { iov_t v[1] = { { sbuf, UEND - (uint64_t)sbuf + 1 } }; s = srv(256, NULL, 0, 16);
      corner("send iov past the end by 1", v, 1, ok_r, 1, &s, &send_fault); }
    { iov_t v[1] = { { sbuf, UEND - (uint64_t)sbuf } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 256, (long long)(UEND - (uint64_t)sbuf), 64 };
      corner("send iov up to the end exactly", v, 1, ok_r, 1, &s, &e); }
    { iov_t v[1] = { { (void *)0x1000, 64 } }; s = srv(256, NULL, 0, 16);
      corner("send iov in unmapped memory", v, 1, ok_r, 1, &s, &send_fault); }
    { iov_t v[1] = { { sbuf, 1ULL << 30 } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 256, 1LL << 30, 64 };
      corner("send iov running into unmapped memory, not taken", v, 1, ok_r, 1, &s, &e); }
    { iov_t v[2] = { { sbuf, 64 }, { (void *)0x1000, 64 } }; s = srv(256, NULL, 0, 16);
      corner("send iov {64} {unmapped 64}, server takes 256", v, 2, ok_r, 1, &s, &send_fault); }
    { iov_t v[2] = { { sbuf, 64 }, { (void *)0x1000, 64 } }; s = srv(64, NULL, 0, 16);
      corner("send iov {64} {unmapped 64}, server takes 64", v, 2, ok_r, 1, &s, &send_fault); }
    { iov_t v[2] = { { sbuf, 64 }, { sbuf, UINT64_MAX } }; s = srv(64, NULL, 0, 16);
      corner("send iov {64} {wraps}, server takes 64", v, 2, ok_r, 1, &s, &send_fault); }

    /* the client's reply buffers */
    { iov_t v[1] = { { ans, UINT64_MAX } }; s = srv(256, NULL, 0, 16);
      struct expect e = { -1, EFAULT, 1, 0, 64, 64, -1 };
      corner("reply iov wraps", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[1] = { { ans, UEND - (uint64_t)ans + 1 } }; s = srv(256, NULL, 0, 16);
      struct expect e = { -1, EFAULT, 1, 0, 64, 64, (long long)(UEND - (uint64_t)ans + 1) };
      corner("reply iov past the end by 1", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[1] = { { ans, UEND - (uint64_t)ans } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 64, 64, (long long)(UEND - (uint64_t)ans) };
      corner("reply iov up to the end exactly", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[1] = { { ans, 1ULL << 30 } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 64, 64, 1LL << 30 };
      corner("reply iov running into unmapped memory, not reached", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[1] = { { (void *)0x1000, 64 } }; s = srv(256, NULL, 0, 16);
      struct expect e = { -1, EFAULT, 1, 0, 64, 64, 64 };
      corner("reply iov in unmapped memory", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[2] = { { ans, 64 }, { (void *)0x1000, 64 } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 64, 64, 128 };
      corner("reply iov {64} {unmapped 64}, reply of 16", ok_s, 1, v, 2, &s, &e); }
    { iov_t v[2] = { { ans, 64 }, { ans, UINT64_MAX } }; s = srv(256, NULL, 0, 16);
      struct expect e = { 5, 0, 1, 0, 64, 64, 63 };
      corner("reply iov {64} {wraps}, reply of 16", ok_s, 1, v, 2, &s, &e); }
    { iov_t v[2] = { { ans, 32 }, { ans + 32, UINT64_MAX } }; s = srv(256, NULL, 0, 64);
      struct expect e = { 5, 0, 1, 0, 64, 64, 31 };
      corner("reply iov {32} {wraps}, reply of 64", ok_s, 1, v, 2, &s, &e); }
    { iov_t v[2] = { { ans, 32 }, { (void *)0x1000, 64 } }; s = srv(256, NULL, 0, 64);
      struct expect e = { -1, EFAULT, 1, 0, 64, 64, 96 };
      corner("reply iov {32} {unmapped 64}, reply of 64", ok_s, 1, v, 2, &s, &e); }
    { iov_t v[1] = { { ans, UEND - (uint64_t)ans + 1 } }; s = srv(256, NULL, 0, 0);
      struct expect e = { 5, 0, 1, 0, 64, 64, NOCHECK };
      corner("reply iov past the end by 1, reply of 0", ok_s, 1, v, 1, &s, &e); }
    { iov_t v[1] = { { ans, UINT64_MAX } }; s = srv(256, NULL, 0, 0);
      struct expect e = { 5, 0, 1, 0, 64, 64, -1 };
      corner("reply iov wraps, reply of 0", ok_s, 1, v, 1, &s, &e); }

    /* the server's receive buffers */
    const struct expect recv_fault = { -1, ESRVRFAULT, 0, EFAULT, 0, 0, 0 };
    { s = srv(UINT64_MAX, NULL, 0, 16);
      corner("receive iov wraps", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(UEND - (uint64_t)rbuf + 1, NULL, 0, 16);
      corner("receive iov past the end by 1", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(UEND - (uint64_t)rbuf, NULL, 0, 16);
      corner("receive iov up to the end exactly", ok_s, 1, ok_r, 1, &s, &served); }
    { s = srv(64, NULL, 0, 16); s.riov[0].iov_base = (void *)0x1000;
      corner("receive iov in unmapped memory", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(1ULL << 30, NULL, 0, 16);
      corner("receive iov running into unmapped memory, not reached", ok_s, 1, ok_r, 1, &s, &served); }
    { s = srv(32, (void *)0x1000, 64, 16);
      corner("receive iov {32} {unmapped 64}, 64-byte message", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(32, (void *)0x1000, 64, 16); iov_t v[1] = { { sbuf, 32 } };
      struct expect e = { 5, 0, 1, 0, 32, 32, 64 };
      corner("receive iov {32} {unmapped 64}, 32-byte message", v, 1, ok_r, 1, &s, &e); }
    { s = srv(256, rbuf, UINT64_MAX, 16);
      corner("receive iov {256} {wraps}, 64-byte message", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(32, rbuf + 32, UINT64_MAX, 16);
      corner("receive iov {32} {wraps}", ok_s, 1, ok_r, 1, &s, &recv_fault); }
    { s = srv(32, rbuf + 32, UEND, 16);
      corner("receive iov {32} {past the end}", ok_s, 1, ok_r, 1, &s, &recv_fault); }

    /* a pulse into a wrapping receive iov is lost with the receive */
    {
        pthread_t t;
        s = srv(UINT64_MAX, NULL, 0, 16);
        s.rcvid = -2;
        pthread_create(&t, NULL, server_thread, &s);
        usleep(20000);
        MsgSendPulse(coid, 10, 42, 0);
        pthread_join(t, NULL);
        check("pulse into a wrapping receive iov: receive fails with EFAULT", s.rcvid == -1 && s.err == EFAULT);
        struct _pulse p;
        memset(&p, 0, sizeof p);
        MsgSendPulse(coid, 10, 43, 0);
        int r = MsgReceivePulse(chid, &p, sizeof p, NULL);
        check("the next receive gets the next pulse, the first one lost", r == 0 && p.code == 43);
    }

    /* MsgReadv() and MsgWritev(): the main thread is the server, a helper the client */
    {
        pthread_t c;
        extern void *client_send(void *);
        pthread_create(&c, NULL, client_send, NULL);
        struct _msg_info info;
        int rcvid = MsgReceive(chid, rbuf, sizeof rbuf, &info);
        check("MsgReadv corners: the message arrived", rcvid > 0);
        static char b[128];
        iov_t iov = { b, 32 };
        errno = 0;
        long n = MsgReadv(rcvid, &iov, 1, UINT64_MAX - 1);
        check("MsgReadv at offset UINT64_MAX - 1 reads 0 bytes", n == 0);
        n = MsgReadv(rcvid, &iov, 1, 1000);
        check("MsgReadv at offset 1000 reads 0 bytes", n == 0);
        n = MsgReadv(rcvid, &iov, 1, 32);
        check("MsgReadv at offset 32 reads 32 bytes", n == 32 && memcmp(b, sbuf + 32, 32) == 0);
        n = MsgWritev(rcvid, &iov, 1, UINT64_MAX - 1);
        check("MsgWritev at offset UINT64_MAX - 1 writes 0 bytes", n == 0);
        iov_t big = { b, UINT64_MAX };
        errno = 0;
        n = MsgReadv(rcvid, &big, 1, 0);
        check("MsgReadv into a wrapping iov: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        errno = 0;
        n = MsgWritev(rcvid, &big, 1, 0);
        check("MsgWritev from a wrapping iov writes what the client takes (64)", n == 64);
        iov_t big2 = { b, 1ULL << 30 };
        n = MsgReadv(rcvid, &big2, 1, 0);
        check("MsgReadv into an iov running into unmapped memory reads the message", n == 64);
        n = MsgWritev(rcvid, &big2, 1, 0);
        check("MsgWritev from an iov running into unmapped memory writes 64", n == 64);
        iov_t two[2] = { { b, 32 }, { (void *)0x1000, 64 } };
        errno = 0;
        n = MsgReadv(rcvid, two, 2, 0);
        check("MsgReadv into {32} {unmapped 64}: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        errno = 0;
        n = MsgWritev(rcvid, two, 2, 0);
        check("MsgWritev from {32} {unmapped 64}: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        iov_t un[1] = { { (void *)0x1000, 64 } };
        errno = 0;
        n = MsgReadv(rcvid, un, 1, 0);
        check("MsgReadv into unmapped memory: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        errno = 0;
        n = MsgWritev(rcvid, un, 1, 0);
        check("MsgWritev from unmapped memory: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        MsgReply(rcvid, 5, rep, 16);
        pthread_join(c, NULL);
    }

    /* /proc/self/as at unmapped and wrapping ranges */
    {
        int fd = open("/proc/self/as", O_RDWR);
        char b[32];
        errno = 0;
        long n = pread(fd, b, sizeof b, (off_t)(UINT64_MAX - 15));
        check("pread(/proc/self/as) at UINT64_MAX - 15: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        errno = 0;
        n = pread(fd, b, sizeof b, (off_t)0x1000);
        check("pread(/proc/self/as) at unmapped 0x1000: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        errno = 0;
        n = pwrite(fd, b, sizeof b, (off_t)(UINT64_MAX - 15));
        check("pwrite(/proc/self/as) at UINT64_MAX - 15: ESRVRFAULT", n == -1 && errno == ESRVRFAULT);
        n = pread(fd, b, sizeof b, (off_t)(uintptr_t)sbuf);
        check("pread(/proc/self/as) of sbuf reads it", n == 32 && memcmp(b, sbuf, 32) == 0);
        close(fd);
    }
    printf("iovtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

void *client_send(void *a)
{
    (void)a;
    MsgSend(coid, sbuf, 64, ans, 64);
    return NULL;
}

/* ConnectDetach() of a connection a thread is blocked on (review IPC-6),
 * as procnto does (measured):
 * - a SEND-blocked thread's send fails with EBADF at once and the server
 *   never sees the message;
 * - a REPLY-blocked thread's send fails with EBADF at once on a channel
 *   without _NTO_CHF_UNBLOCK, and its rcvid is dead (MsgReply(): ESRCH);
 * - on a channel with _NTO_CHF_UNBLOCK the server gets
 *   _PULSE_CODE_UNBLOCK with the rcvid, the thread waits, and the
 *   server's MsgReply() ends the send with its status; the connection's
 *   _PULSE_CODE_DISCONNECT (with _NTO_CHF_DISCONNECT) comes after the
 *   reply, not before.
 * Prints one "ok"/"FAIL" line per check and "cdetachtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;
static int chid, coid;
static volatile long sr;
static volatile int serr;
static char sbuf[32] = "hello", ans[32];

static void check(const char *what, int ok)
{
    printf("cdetachtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *sender(void *a)
{
    (void)a;
    errno = 0;
    sr = MsgSend(coid, sbuf, 32, ans, 32);
    serr = errno;
    return NULL;
}

/* the next thing on the channel within 300 ms: a pulse's code, a message's
 * rcvid (> 0), or -1 for nothing */
static int next(int *value, int *scoid)
{
    char buf[64];
    struct _msg_info info;
    uint64_t to = 300000000ULL;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &to, NULL);
    int rcvid = MsgReceive(chid, buf, sizeof buf, &info);
    if (rcvid == 0) {
        struct _pulse *p = (struct _pulse *)buf;
        *value = p->value.sival_int;
        *scoid = p->scoid;
        return p->code;
    }
    if (rcvid > 0) {
        *scoid = info.scoid;
    }
    return rcvid;
}

int main(void)
{
    pthread_t t;
    int value, scoid, r;
    char buf[64];
    struct _msg_info info;

    /* a plain channel */
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    sr = -2;
    pthread_create(&t, NULL, sender, NULL);
    usleep(30000);
    check("plain: ConnectDetach() with a SEND-blocked thread", ConnectDetach(coid) == 0);
    usleep(30000);
    check("plain: the sender failed with EBADF at once", sr == -1 && serr == EBADF);
    check("plain: the server never sees the message", next(&value, &scoid) == -1);
    pthread_join(t, NULL);

    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    sr = -2;
    pthread_create(&t, NULL, sender, NULL);
    usleep(30000);
    int rcvid = MsgReceive(chid, buf, sizeof buf, &info);
    check("plain: the message received", rcvid > 0);
    check("plain: ConnectDetach() with a REPLY-blocked thread", ConnectDetach(coid) == 0);
    usleep(30000);
    check("plain: the REPLY-blocked sender failed with EBADF at once", sr == -1 && serr == EBADF);
    check("plain: no pulse for the server", next(&value, &scoid) == -1);
    errno = 0;
    r = MsgReply(rcvid, 9, "bye", 4);
    check("plain: MsgReply() of the dead rcvid is ESRCH", r == -1 && errno == ESRCH);
    pthread_join(t, NULL);
    ChannelDestroy(chid);

    /* _NTO_CHF_UNBLOCK | _NTO_CHF_DISCONNECT */
    chid = ChannelCreate(_NTO_CHF_UNBLOCK | _NTO_CHF_DISCONNECT);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    sr = -2;
    pthread_create(&t, NULL, sender, NULL);
    usleep(30000);
    check("unblock: ConnectDetach() with a SEND-blocked thread", ConnectDetach(coid) == 0);
    usleep(30000);
    check("unblock: the SEND-blocked sender failed with EBADF at once", sr == -1 && serr == EBADF);
    check("unblock: the server gets the disconnect pulse, no message",
          next(&value, &scoid) == _PULSE_CODE_DISCONNECT && next(&value, &scoid) == -1);
    pthread_join(t, NULL);
    ConnectDetach(scoid | 0x10000);

    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    sr = -2;
    pthread_create(&t, NULL, sender, NULL);
    usleep(30000);
    rcvid = MsgReceive(chid, buf, sizeof buf, &info);
    check("unblock: the message received", rcvid > 0);
    check("unblock: ConnectDetach() with a REPLY-blocked thread", ConnectDetach(coid) == 0);
    usleep(30000);
    check("unblock: the sender still waits", sr == -2);
    r = next(&value, &scoid);
    check("unblock: the server gets the unblock pulse with the rcvid", r == _PULSE_CODE_UNBLOCK && value == rcvid);
    check("unblock: no disconnect pulse before the reply", next(&value, &scoid) == -1);
    check("unblock: MsgReply() of the rcvid works", MsgReply(rcvid, 9, "bye", 4) == 0);
    pthread_join(t, NULL);
    check("unblock: the sender got the reply's status", sr == 9);
    check("unblock: the disconnect pulse comes after the reply", next(&value, &scoid) == _PULSE_CODE_DISCONNECT);
    errno = 0;
    r = MsgError(rcvid, EIO);
    check("unblock: MsgError() of the replied rcvid is ESRCH", r == -1 && errno == ESRCH);
    ChannelDestroy(chid);
    printf("cdetachtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

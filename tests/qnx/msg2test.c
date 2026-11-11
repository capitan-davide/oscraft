/* Message passing details as procnto gives them:
 * - struct _msg_info: msglen (what was received), srcmsglen, dstmsglen,
 *   flags (_NTO_MI_BITS_64); MsgInfo() later gives the same msglen;
 * - MsgRead()/MsgWrite() at and past the ends of the buffers;
 * - a second MsgReply(): ESRCH;
 * - channel notifications: _PULSE_CODE_DISCONNECT when a client detaches
 *   or ends, _PULSE_CODE_THREADDEATH (_NTO_CHF_THREAD_DEATH) when a thread
 *   of the process ends, _PULSE_CODE_COIDDEATH (_NTO_CHF_COID_DISCONNECT)
 *   for each connection to a server that ends;
 * - a private channel takes no other process's connections (EPERM);
 * - MsgCurrent(): 0 for a message received and not replied to, else
 *   ESRCH; MsgPause(): ENOSYS; ChannelCreatePulsePool(); channel flags;
 * - TimerTimeout() on a send that queues: with _NTO_TIMEOUT_SEND,
 *   _NTO_TIMEOUT_REPLY or both it ends the send with ETIMEDOUT at the
 *   deadline, whether the client is still SEND-blocked or already
 *   REPLY-blocked by then; a send the server was waiting for starts in
 *   REPLY and only _NTO_TIMEOUT_REPLY covers it (review item IPC-3).
 * Prints one "ok"/"FAIL" line per check and "msg2test: passed". */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static int chid;

static void check(const char *what, int ok)
{
    printf("msg2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Sends 100 bytes, takes a 40-byte reply, detaches. */
static void *client(void *arg)
{
    (void)arg;
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    char s[100], r[40];
    memset(s, 'a', sizeof s);
    long n = MsgSend(coid, s, sizeof s, r, sizeof r);
    check("MsgSend() returns the reply's status", n == 77);
    ConnectDetach(coid);
    return NULL;
}

static void *nothing(void *arg)
{
    return arg;
}

/* The next pulse on `ch` within 200 ms: its code (0 if none), its value
 * in `*value`. */
static int next_pulse(int ch, int *value)
{
    struct _pulse p;
    memset(&p, 0, sizeof p);
    uint64_t ns = 200000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
    if (MsgReceive(ch, &p, sizeof p, NULL) != 0) {
        return 0;
    }
    *value = p.value.sival_int;
    return p.code;
}

static void messages(void)
{
    chid = ChannelCreate(_NTO_CHF_DISCONNECT | _NTO_CHF_THREAD_DEATH | _NTO_CHF_SENDER_LEN | _NTO_CHF_PRIVATE);
    pthread_t t;
    pthread_create(&t, NULL, client, NULL);
    char buf[16];
    struct _msg_info info;
    int rcvid = MsgReceive(chid, buf, sizeof buf, &info);
    check("_msg_info: msglen what was received, srcmsglen, dstmsglen",
          rcvid > 0 && info.msglen == 16 && info.srcmsglen == 100 && info.dstmsglen == 40);
    check("_msg_info: flags _NTO_MI_BITS_64", info.flags == _NTO_MI_BITS_64);
    check("MsgRead() of the last 5 bytes", MsgRead(rcvid, buf, sizeof buf, 95) == 5);
    check("MsgRead() at the end: 0", MsgRead(rcvid, buf, sizeof buf, 100) == 0);
    check("MsgRead() past the end: 0", MsgRead(rcvid, buf, sizeof buf, 200) == 0);
    check("MsgWrite() at the reply's last 2 bytes", MsgWrite(rcvid, "xyz", 3, 38) == 2);
    check("MsgWrite() past the end: 0", MsgWrite(rcvid, "xyz", 3, 50) == 0);
    struct _msg_info later;
    check("MsgInfo(): the msglen received", MsgInfo(rcvid, &later) == 0 && later.msglen == 16);
    check("MsgReply()", MsgReply(rcvid, 77, "reply", 5) == 0);
    errno = 0;
    check("MsgReply() again: ESRCH", MsgReply(rcvid, 77, "reply", 5) == -1 && errno == ESRCH);
    pthread_join(t, NULL);

    int value = -1;
    check("a client detaches: _PULSE_CODE_DISCONNECT", next_pulse(chid, &value) == _PULSE_CODE_DISCONNECT);
    check("its thread ends: _PULSE_CODE_THREADDEATH, the thread ID",
          next_pulse(chid, &value) == _PULSE_CODE_THREADDEATH && value == 2);
    check("then nothing", next_pulse(chid, &value) == 0);
    pthread_create(&t, NULL, nothing, NULL);
    pthread_join(t, NULL);
    check("another thread ends: _PULSE_CODE_THREADDEATH", next_pulse(chid, &value) == _PULSE_CODE_THREADDEATH && value == 2);
}

static void server_death(void)
{
    int notify = ChannelCreate(_NTO_CHF_COID_DISCONNECT | _NTO_CHF_PRIVATE);
    int p[2];
    pipe(p);
    pid_t pid = fork();
    if (pid == 0) {
        int chids[2] = { ChannelCreate(_NTO_CHF_PRIVATE), ChannelCreate(0) };
        write(p[1], chids, sizeof chids);
        sleep(1);
        _exit(0);
    }
    int chids[2];
    read(p[0], chids, sizeof chids);
    errno = 0;
    check("another process's private channel: EPERM",
          ConnectAttach(0, pid, chids[0], _NTO_SIDE_CHANNEL, 0) == -1 && errno == EPERM);
    int side = ConnectAttach(0, pid, chids[1], _NTO_SIDE_CHANNEL, 0);
    int fd = ConnectAttach(0, pid, chids[1], 0, 0);
    waitpid(pid, NULL, 0);
    int value = -1;
    check("the server ends: _PULSE_CODE_COIDDEATH for the descriptor first",
          next_pulse(notify, &value) == _PULSE_CODE_COIDDEATH && value == fd);
    check("then for the side channel connection",
          next_pulse(notify, &value) == _PULSE_CODE_COIDDEATH && value == side);
    check("then nothing", next_pulse(notify, &value) == 0);
    char b[4];
    errno = 0;
    check("a send to the dead server: EBADF", MsgSend(side, b, 4, b, 4) == -1 && errno == EBADF);
    check("the connection detaches", ConnectDetach(side) == 0);
}

static void *one_send(void *arg)
{
    int coid = ConnectAttach(0, 0, *(int *)arg, _NTO_SIDE_CHANNEL, 0);
    char b[4] = "abc";
    MsgSend(coid, b, 4, b, 4);
    ConnectDetach(coid);
    return NULL;
}

static void current_pause_pools(void)
{
    int ch = ChannelCreate(_NTO_CHF_PRIVATE);
    pthread_t t;
    pthread_create(&t, NULL, one_send, &ch);
    char b[8];
    int rcvid = MsgReceive(ch, b, sizeof b, NULL);
    check("MsgCurrent() of a message received", MsgCurrent(rcvid) == 0);
    errno = 0;
    check("MsgPause(): ENOSYS", MsgPause(rcvid, 7) == -1 && errno == ENOSYS);
    MsgReply(rcvid, 0, NULL, 0);
    pthread_join(t, NULL);
    errno = 0;
    check("MsgCurrent() once replied: ESRCH", MsgCurrent(rcvid) == -1 && errno == ESRCH);
    errno = 0;
    check("MsgCurrent(0): ESRCH", MsgCurrent(0) == -1 && errno == ESRCH);
    struct nto_channel_config cfg;
    memset(&cfg, 0, sizeof cfg);
    SIGEV_NONE_INIT(&cfg.event);
    cfg.num_pulses = 4;
    cfg.rearm_threshold = 2;
    check("ChannelCreatePulsePool()", ChannelCreatePulsePool(0, &cfg) > 0);
    errno = 0;
    check("ChannelCreatePulsePool(no config): EFAULT", ChannelCreatePulsePool(0, NULL) == -1 && errno == EFAULT);
    cfg.num_pulses = 0;
    errno = 0;
    check("ChannelCreatePulsePool(no pulses): EINVAL", ChannelCreatePulsePool(0, &cfg) == -1 && errno == EINVAL);
    check("_NTO_CHF_MSG_PAUSING, _NTO_CHF_INHERIT_RUNMASK",
          ChannelCreate(_NTO_CHF_MSG_PAUSING) > 0 && ChannelCreate(_NTO_CHF_INHERIT_RUNMASK) > 0);
    errno = 0;
    check("_NTO_CHF_UNBLOCK_TIMER: EINVAL", ChannelCreate(_NTO_CHF_UNBLOCK_TIMER) == -1 && errno == EINVAL);
}

static int tmo_chid, tmo_coid;
static volatile int tmo_receive_after_ms, tmo_reply_after_ms, tmo_waiting;

/* Receives tmo_receive_after_ms after it is told to, replies
 * tmo_reply_after_ms later; gives up on a client that went away. */
static void *tmo_server(void *arg)
{
    (void)arg;
    char buf[16];
    for (;;) {
        int rcv = tmo_receive_after_ms, rep = tmo_reply_after_ms;
        if (rcv < 0) {
            usleep(5000);
            continue;
        }
        usleep(rcv * 1000);
        uint64_t ns = 600000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
        tmo_waiting = 1;
        int rcvid = MsgReceive(tmo_chid, buf, sizeof buf, NULL);
        tmo_waiting = 0;
        if (rcvid > 0) {
            usleep(rep * 1000);
            MsgReply(rcvid, 0, "ok", 2);
        }
        tmo_receive_after_ms = -1;
    }
    return NULL;
}

/* A send with a 100 ms TimerTimeout of `flags`, the server receiving
 * `rcv_ms` in and replying `rep_ms` after that: MsgSend()'s result. */
static int timed_send(unsigned flags, int rcv_ms, int rep_ms, int *err)
{
    char rep[16];
    uint64_t ns = 100000000;
    tmo_reply_after_ms = rep_ms;
    tmo_receive_after_ms = rcv_ms;
    if (rcv_ms == 0) {
        /* "the server is waiting" must be true, not nearly true: a send
         * that blocks in SEND for an instant is a different case */
        while (!tmo_waiting) {
            usleep(1000);
        }
        usleep(20000);
    } else {
        usleep(5000);
    }
    TimerTimeout(CLOCK_MONOTONIC, flags, NULL, &ns, NULL);
    errno = 0;
    int r = MsgSend(tmo_coid, "hi", 2, rep, sizeof rep);
    *err = r < 0 ? errno : 0;
    while (tmo_receive_after_ms >= 0) {
        usleep(5000);
    }
    return r;
}

static void send_timeouts(void)
{
    pthread_t t;
    int err;
    tmo_chid = ChannelCreate(0);
    tmo_coid = ConnectAttach(0, 0, tmo_chid, _NTO_SIDE_CHANNEL, 0);
    tmo_receive_after_ms = -1;
    pthread_create(&t, NULL, tmo_server, NULL);
    usleep(10000);
    /* the server receives at 200 ms: the send is still queued at the
     * deadline, whatever the bits */
    check("SEND|REPLY, still SEND-blocked at the deadline: ETIMEDOUT",
          timed_send(_NTO_TIMEOUT_SEND | _NTO_TIMEOUT_REPLY, 200, 200, &err) == -1 && err == ETIMEDOUT);
    check("SEND only, still SEND-blocked: ETIMEDOUT", timed_send(_NTO_TIMEOUT_SEND, 200, 200, &err) == -1 && err == ETIMEDOUT);
    check("REPLY only, still SEND-blocked: ETIMEDOUT", timed_send(_NTO_TIMEOUT_REPLY, 200, 200, &err) == -1 && err == ETIMEDOUT);
    /* received at 20 ms, replied at 320: REPLY-blocked at the deadline */
    check("SEND|REPLY, REPLY-blocked by the deadline: ETIMEDOUT",
          timed_send(_NTO_TIMEOUT_SEND | _NTO_TIMEOUT_REPLY, 20, 300, &err) == -1 && err == ETIMEDOUT);
    check("SEND only, REPLY-blocked by the deadline: ETIMEDOUT too", timed_send(_NTO_TIMEOUT_SEND, 20, 300, &err) == -1 && err == ETIMEDOUT);
    check("REPLY only, REPLY-blocked by the deadline: ETIMEDOUT", timed_send(_NTO_TIMEOUT_REPLY, 20, 300, &err) == -1 && err == ETIMEDOUT);
    check("replied at 40 ms: no timeout", timed_send(_NTO_TIMEOUT_SEND | _NTO_TIMEOUT_REPLY, 20, 20, &err) == 0);
    /* the server already waiting: REPLY-blocked from the start */
    check("REPLY only, server waiting, reply at 300: ETIMEDOUT", timed_send(_NTO_TIMEOUT_REPLY, 0, 300, &err) == -1 && err == ETIMEDOUT);
    check("SEND only, server waiting, reply at 300: no timeout", timed_send(_NTO_TIMEOUT_SEND, 0, 300, &err) == 0);
    ConnectDetach(tmo_coid);
    ChannelDestroy(tmo_chid);
}

int main(void)
{
    messages();
    server_death();
    current_pause_pools();
    send_timeouts();
    printf("msg2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

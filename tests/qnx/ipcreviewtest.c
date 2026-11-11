/* Message passing details of review #184, as procnto answers (measured):
 * - IPC-9: a pulse received into a buffer shorter than struct _pulse:
 *   EFAULT, and the pulse is gone;
 * - IPC-13: MsgSendPulse() to a channel ID the caller doesn't have
 *   (-chid): ESRCH;
 * - IPC-14: a _PULSE_CODE_DISCONNECT and a _PULSE_CODE_COIDDEATH come at
 *   the priority of the process that ended (the receiver then runs at it);
 * - IPC-18: ability IDs <sys/procmgr.h> leaves undefined: nobody has
 *   them, root included;
 * - IPC-22: a dupfd registration imports once (then ENOENT); those for a
 *   receiver that ended don't count against the sender's 200;
 * - IPC-23: a message of 1025 or 4096 iov parts.
 * Prints one "ok"/"FAIL" line per check and "ipcreviewtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/dupfd.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("ipcreviewtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int chid;

static void *echo_server(void *a)
{
    (void)a;
    static char buf[8192];
    for (;;) {
        int r = MsgReceive(chid, buf, sizeof buf, NULL);
        if (r > 0)
            MsgReply(r, 0, buf, 8);
    }
    return NULL;
}

static int curprio(void)
{
    struct sched_param p;
    SchedGet(0, 0, &p);
    return p.sched_curpriority;
}

static void pulses(void)
{
    int ch = ChannelCreate(0), co = ConnectAttach(0, 0, ch, _NTO_SIDE_CHANNEL, 0);
    MsgSendPulse(co, 10, 5, 77);
    char small[8];
    int r = MsgReceive(ch, small, sizeof small, NULL);
    check("a pulse into 8 bytes: EFAULT", r == -1 && errno == EFAULT);
    struct _pulse pl;
    uint64_t ns = 50000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
    r = MsgReceivePulse(ch, &pl, sizeof pl, NULL);
    check("and it is gone", r == -1 && errno == ETIMEDOUT);
    errno = 0;
    check("MsgSendPulse() to a channel there isn't: ESRCH", MsgSendPulse(-99, 10, 1, 0) == -1 && errno == ESRCH);
    ConnectDetach(co);
    ChannelDestroy(ch);
}

static void kernel_pulses(void)
{
    struct sched_param sp = {.sched_priority = 50};
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    int dch = ChannelCreate(_NTO_CHF_DISCONNECT);
    pid_t k = fork();
    if (k == 0) {
        sp.sched_priority = 12;
        pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
        ConnectAttach(0, getppid(), dch, _NTO_SIDE_CHANNEL, 0);
        _exit(0);
    }
    waitpid(k, NULL, 0);
    struct _pulse pl;
    MsgReceivePulse(dch, &pl, sizeof pl, NULL);
    check("DISCONNECT at the ended client's priority (12)", pl.code == _PULSE_CODE_DISCONNECT && curprio() == 12);
    sp.sched_priority = 50;
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    int cch = ChannelCreate(_NTO_CHF_COID_DISCONNECT);
    int p2c[2];
    pipe(p2c);
    k = fork();
    if (k == 0) {
        sp.sched_priority = 12;
        pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
        int ch = ChannelCreate(0);
        write(p2c[1], &ch, sizeof ch);
        sleep(1);
        _exit(0);
    }
    int kch;
    read(p2c[0], &kch, sizeof kch);
    ConnectAttach(0, k, kch, 0, 0);
    waitpid(k, NULL, 0);
    MsgReceivePulse(cch, &pl, sizeof pl, NULL);
    check("COIDDEATH at the ended server's priority (12)", pl.code == _PULSE_CODE_COIDDEATH && curprio() == 12);
    sp.sched_priority = 10;
    pthread_setschedparam(pthread_self(), SCHED_RR, &sp);
}

static void abilities(void)
{
    int ch = ChannelCreate(0);
    pid_t k = fork();
    if (k == 0) {
        int c = ConnectAttach(0, getppid(), ch, _NTO_SIDE_CHANNEL, 0);
        char b;
        MsgSend(c, "x", 1, &b, 1);
        _exit(0);
    }
    struct _msg_info mi;
    char b;
    int rc = MsgReceive(ch, &b, 1, &mi);
    unsigned ids[] = {21, 25, 30, 49, 55, 63, 64};
    int want_able[] = {1, 0, 0, 0, 0, 0, 1};
    int ok = 1;
    for (unsigned i = 0; i < sizeof ids / sizeof ids[0]; i++) {
        struct _client_able a = {.ability = ids[i]};
        struct _client_info *ip = NULL;
        int r = ConnectClientInfoAble(mi.scoid, &ip, 0, &a, 1);
        ok &= r == 0 && ip && (!(ip->flags & _NTO_CI_UNABLE)) == want_able[i];
        free(ip);
    }
    check("undefined ability IDs (25, 30, 49, 55, 63): unable, even for root", ok);
    MsgReply(rc, 0, &b, 1);
    waitpid(k, NULL, 0);
}

static void dup_registrations(void)
{
    int fd = open("/dev/null", O_RDWR);
    int p2c[2], c2p[2];
    pipe(p2c);
    pipe(c2p);
    pid_t k = fork();
    if (k == 0) {
        dupfd_handle_t h;
        read(p2c[0], &h, sizeof h);
        int a = dupfd(h, 0);
        int ea = a >= 0 ? 0 : errno;
        int c = dupfd(h, 0);
        int res[2] = {ea, c >= 0 ? 0 : errno};
        write(c2p[1], res, sizeof res);
        _exit(0);
    }
    dupfd_handle_t h;
    dupfd_register(getpid64(k), fd, 0, &h);
    write(p2c[1], &h, sizeof h);
    int res[2];
    read(c2p[0], res, sizeof res);
    check("dupfd: one import per registration (then ENOENT)", res[0] == 0 && res[1] == ENOENT);
    waitpid(k, NULL, 0);
    k = fork();
    if (k == 0) {
        sleep(1);
        _exit(0);
    }
    int m = 0;
    while (m < 250 && dupfd_register(getpid64(k), fd, DUPFD_NO_DISCARD, &h) == 0)
        m++;
    check("and one for a receiver that ended doesn't count: 200 more", m == 200 && errno == EAGAIN);
    waitpid(k, NULL, 0);
    close(fd);
}

static void many_parts(void)
{
    chid = ChannelCreate(0);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    pthread_t srv;
    pthread_create(&srv, NULL, echo_server, NULL);
    static iov_t iov[4096];
    static char bytes[4096];
    for (int i = 0; i < 4096; i++)
        SETIOV(&iov[i], &bytes[i], 1);
    char rep[8];
    check("a message of 1025 parts", MsgSendvs(coid, iov, 1025, rep, 8) == 0);
    check("and of 4096", MsgSendvs(coid, iov, 4096, rep, 8) == 0);
}

int main(void)
{
    pulses();
    kernel_pulses();
    abilities();
    dup_registrations();
    many_parts();
    printf("ipcreviewtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

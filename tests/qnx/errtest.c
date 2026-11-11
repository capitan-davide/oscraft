/* Kernel calls' errors as procnto gives them: each call with invalid
 * arguments, and the result QNX 8's procnto returns (measured in the
 * reference image; the expectations below are its results). Prints one
 * "ok"/"FAIL" line per call and "errtest: passed". */
#include <stdio.h>
#include <errno.h>
#include <signal.h>
#include <time.h>
#include <sched.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/mman.h>
#include <sys/procmsg.h>
#include <unistd.h>

static int fails;

/* `expr` fails with errno `err`. */
#define E(expr, err)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == -1 && errno == (err);                                                          \
        printf("errtest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);              \
        fails += !ok_;                                                                                 \
    } while (0)
/* `expr` gives `val`. */
#define V(expr, val)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == (long)(val);                                                                   \
        printf("errtest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);              \
        fails += !ok_;                                                                                 \
    } while (0)

int main(void)
{
    char buf[64];
    struct _msg_info info;
    uint64_t t64;
    sync_t s;
    memset(&s, 0, sizeof s);
    struct sigevent ev;
    SIGEV_NONE_INIT(&ev);
    struct _clockadjust adj = { 0 };
    struct _clockperiod per = { 0 };
    struct sched_param sp = { 0 };
    struct _sched_info si;
    struct _client_info ci;
    struct _server_info svi;
    int chid = ChannelCreate(0);
    signal(SIGUSR1, SIG_IGN);
    E(ChannelCreate(0x80000000), EINVAL);
    E(ChannelDestroy(999), ESRCH);
    E(ChannelDestroy(-1), ESRCH);
    E(ConnectAttach(0, 99999, 1, 0, 0), ESRCH);
    E(ConnectAttach(0, 0, 999, 0, 0), ESRCH);
    V(ConnectAttach(5, 0, chid, 0, 0), 0x3);
    /* the index: a descriptor at or above RLIMIT_NOFILE (1000) is
     * EMFILE, one too big to be a descriptor EINVAL; a side channel's
     * index is ignored (review item IPC-1: the kernel used to size a
     * table by it) */
    E(ConnectAttach(0, 0, chid, 1000, 0), EMFILE);
    E(ConnectAttach(0, 0, chid, 32768, 0), EMFILE);
    E(ConnectAttach(0, 0, chid, 65535, 0), EINVAL);
    E(ConnectAttach(0, 0, chid, 0x3fffffff, 0), EINVAL);
    V(ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL + 0x3fffffff, 0), _NTO_SIDE_CHANNEL + 1);
    V(ConnectDetach(_NTO_SIDE_CHANNEL + 1), 0);
    /* dup2() in the kernel: a target at or above the limit is EMFILE,
     * one too big to be a descriptor EBADF (libc's dup2() says EBADF
     * for both, before the kernel sees them) */
    E(ConnectAttach(0, 0, 0, 1500, _NTO_COF_DUP2), EMFILE);
    E(ConnectAttach(0, 0, 0, 0x3fffffff, _NTO_COF_DUP2), EBADF);
    E(ConnectDetach(999), EINVAL);
    E(ConnectDetach(-5), EINVAL);
    E(MsgSend(999, buf, 4, buf, 4), EBADF);
    E(MsgSend(-1, buf, 4, buf, 4), EBADF);
    E(MsgReply(12345, 0, buf, 4), ESRCH);
    E(MsgReply(0, 0, buf, 4), ESRCH);
    E(MsgError(12345, 5), ESRCH);
    E(MsgReceive(999, buf, 4, &info), ESRCH);
    E(MsgRead(12345, buf, 4, 0), ESRCH);
    E(MsgWrite(12345, buf, 4, 0), ESRCH);
    E(MsgInfo(12345, &info), ESRCH);
    E(MsgDeliverEvent(12345, &ev), ESRCH);
    E(MsgSendPulse(999, 10, 1, 0), EBADF);
    V(MsgSendPulse(-1, 10, 1, 0), 0x0);
    E(SignalKill(0, 99999, 0, SIGUSR1, 0, 0), ESRCH);
    E(SignalKill(0, 0, 0, 200, 0, 0), EINVAL);
    V(SignalKill(0, 0, 999, SIGUSR1, 0, 0), 0x0);
    E(SignalAction(0, 0, 200, 0, 0), EINVAL);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_flags = 0x40;
    E(SignalAction(0, 0, SIGUSR2, &sa, 0), ENOTSUP);
    sa.sa_flags = 0;
    sa.sa_handler = SIG_IGN;
    E(SignalAction(0, 0, SIGKILL, &sa, 0), EINVAL);
    sa.sa_handler = SIG_DFL;
    V(SignalAction(0, 0, SIGKILL, &sa, 0), 0);
    V(SignalProcmask(0, 0, 99, 0, 0), 0x0);
    E(SignalProcmask(0, 999, SIG_BLOCK, 0, 0), ESRCH);
    E(ThreadDetach(999), ESRCH);
    E(ThreadJoin(999, 0), ESRCH);
    E(ThreadCancel(999, 0), ESRCH);
    E(ThreadCtl(999, 0), EINVAL);
    E(ThreadDestroy(999, -1, 0), ESRCH);
    E(ClockTime(99, 0, &t64), EINVAL);
    E(ClockTime(CLOCK_MONOTONIC, &t64, 0), EINVAL);
    E(ClockAdjust(99, &adj, 0), EINVAL);
    E(ClockPeriod(99, &per, 0, 0), EINVAL);
    E(ClockId(99999, 1), ESRCH);
    E(ClockId(0, 999), ESRCH);
    E(TimerCreate(99, &ev), EINVAL);
    E(TimerDestroy(999), EINVAL);
    E(TimerDestroy(-1), EINVAL);
    E(TimerSettime(999, 0, 0, 0), EFAULT);
    E(TimerInfo(99999, 0, 0, 0), ESRCH);
    E(TimerInfo(0, 999, 0, 0), EINVAL);
    E(TimerAlarm(99, 0, 0), EINVAL);
    E(SyncTypeCreate(99, &s, 0), EINVAL);
    V(SyncDestroy(&s), 0x0);
    E(SyncSemPost(&s), EINVAL);
    E(SyncSemWait(&s, 1), EINVAL);
    E(SyncMutexUnlock(&s), EINVAL);
    E(SyncCondvarSignal(&s, 0), EINVAL);
    E(SyncCtl(99, &s, 0), ENOSYS);
    E(SchedGet(0, 999, &sp), ESRCH);
    E(SchedSet(0, 0, 99, &sp), EINVAL);
    sp.sched_priority = 300;
    E(SchedSet(0, 0, SCHED_FIFO, &sp), EINVAL);
    sp.sched_priority = 0;
    E(SchedSet(0, 0, SCHED_FIFO, &sp), EINVAL);
    E(SchedInfo(0, 99, &si), EINVAL);
    E(SchedInfo(99999, SCHED_FIFO, &si), ESRCH);
    E(SchedCtl(999, 0, 0), ENOSYS);
    E(ConnectClientInfo(9999, &ci, 0), EINVAL);
    V(ConnectServerInfo(0, 999, &svi), 0x40000000);
    E(ConnectServerInfo(99999, 1, &svi), ESRCH);
    E(ConnectFlags(0, 999, 1, 1), EBADF);
    E(InterruptMask(0, 999), ESRCH);
    E(InterruptUnmask(0, 999), ESRCH);
    E(InterruptDetach(999), EINVAL);
    E(InterruptWait(0, 0) == 0 ? 0 : -1, ENODEV);
    E(mmap(0, 0, PROT_READ, MAP_PRIVATE|MAP_ANON, -1, 0) == MAP_FAILED ? -1 : 0, EINVAL);
    E(mmap(0, 4096, PROT_READ, MAP_PRIVATE, 999, 0) == MAP_FAILED ? -1 : 0, EBADF);
    E(mmap(0, 4096, PROT_READ, 0, -1, 0) == MAP_FAILED ? -1 : 0, EINVAL);
    V(mmap((void*)1, 4096, PROT_READ, MAP_PRIVATE|MAP_ANON|MAP_FIXED, -1, 0) == MAP_FAILED ? -1 : 0, 0x0);
    V(munmap((void*)1, 4096), 0x0);
    E(munmap(0, 0), EINVAL);
    E(mprotect((void*)0x1000, 4096, PROT_READ), ENOMEM);
    E(msync((void*)1, 4096, MS_SYNC), ENOMEM);
    E(mlock((void*)0x1000, 4096), ENOMEM);
    /* Messages to procnto shorter than a header (review item PROCMGR-11),
     * each after a valid request whose type stays in procnto's buffer:
     * under 2 bytes EINVAL, 2 or 3 bytes EBADMSG (measured). */
    {
        unsigned char m[4] = { 0x01, 0x00, 0x02, 0x00 };
        char r[64];
        for (int len = 0; len <= 3; len++) {
            getpgid(0);
            E(MsgSend(SYSMGR_COID, m, len, r, sizeof r), len < 2 ? EINVAL : EBADMSG);
        }
    }
    printf("errtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

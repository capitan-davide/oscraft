/* The priority an unprivileged server takes from a privileged client
 * (review item IPC-11 (a)), as procnto (measured): capped at the highest
 * the server's process may take, 63 without PROCMGR_AID_PRIORITY, on a
 * receive and on a server boost (a send while the server is busy). The
 * kernel ran the server at the client's priority (100). A root server
 * still takes 100. Prints one "ok"/"FAIL" line per check and
 * "msgcaptest: passed". */
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static uint64_t cps;

static void check(const char *what, int got, int want)
{
    printf("msgcaptest: %s %s: %d (want %d)\n", got == want ? "ok" : "FAIL", what, got, want);
    fflush(stdout);
    fails += got != want;
}

static int cur(void)
{
    struct sched_param p;
    int pol;
    pthread_getschedparam(pthread_self(), &pol, &p);
    return p.sched_curpriority;
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

/* A server (uid `uid`) receives a message from this process at 100, then,
 * busy, is boosted by a second one. Returns its exit status. */
static int run(int uid, int want)
{
    int fd[2];
    pipe(fd);
    pid_t c = fork();
    if (c == 0) {
        if (uid) {
            setgid(uid);
            setuid(uid);
        }
        setp(10);
        int ch = ChannelCreate(0);
        write(fd[1], &ch, sizeof ch);
        char m;
        int rcvid = MsgReceive(ch, &m, 1, NULL);
        check(uid ? "uid 100 server: received from a client at 100" : "root server: received from a client at 100",
              cur(), want);
        MsgReply(rcvid, 0, &m, 1);
        setp(10);
        write(fd[1], &ch, sizeof ch);
        spin_ms(100); /* the second message comes meanwhile */
        check(uid ? "uid 100 server: boosted while busy" : "root server: boosted while busy", cur(), want);
        rcvid = MsgReceive(ch, &m, 1, NULL);
        MsgReply(rcvid, 0, &m, 1);
        _exit(fails != 0);
    }
    int ch;
    read(fd[0], &ch, sizeof ch);
    setp(100);
    int coid = ConnectAttach(0, c, ch, _NTO_SIDE_CHANNEL, 0);
    char m = 1, r;
    MsgSend(coid, &m, 1, &r, 1);
    read(fd[0], &ch, sizeof ch);
    usleep(20000);
    MsgSend(coid, &m, 1, &r, 1);
    setp(10);
    int st;
    waitpid(c, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : 1;
}

int main(void)
{
    cps = SYSPAGE_ENTRY(qtime)->cycles_per_sec;
    fails += run(100, 63);
    fails += run(0, 100);
    printf("msgcaptest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

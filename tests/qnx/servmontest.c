/* Server monitors (_PROC_SERVMON, QNX's server-monitor), as procnto:
 * ALIVE and IGNORE without a monitor are ESRCH; CONFIG takes
 * PROCMGR_AID_SERVER_MONITOR and there is one monitor at most (EEXIST);
 * IGNORE from another process is EINVAL. A client still waiting for a
 * server with _NTO_CHF_UNBLOCK `timeout` ms after asking to leave makes
 * the monitor's event come, with the client's pid and tid in its value;
 * a server the monitor ignores doesn't hold its clients.
 * Prints one "ok"/"FAIL" line per check and "servmontest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/procmsg.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("servmontest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long servmon(int sub, uint32_t timeout, pid_t target, const struct sigevent *ev)
{
    proc_servmon_t m;
    memset(&m, 0, sizeof m);
    m.i.type = _PROC_SERVMON;
    m.i.subtype = sub;
    m.i.timeout = timeout;
    m.i.target_pid = target;
    if (ev) {
        m.i.event = *ev;
    }
    return MsgSend(PROCMGR_COID, &m.i, sizeof m.i, NULL, 0) == -1 ? -errno : 0;
}

static uint64_t now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000ULL + ts.tv_nsec / 1000000;
}

static volatile int *srv_chid;
static pid_t server;
static int client_tid, client_err;
static uint64_t client_ms;

/* A client that asks to leave after 50 ms (a REPLY timeout). */
static void *client(void *arg)
{
    client_tid = gettid();
    int coid = ConnectAttach(0, server, *srv_chid, _NTO_SIDE_CHANNEL, 0);
    struct sigevent to;
    SIGEV_UNBLOCK_INIT(&to);
    uint64_t ns = 50000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_REPLY, &to, &ns, NULL);
    char c = 0;
    uint64_t t0 = now_ms();
    client_err = MsgSend(coid, &c, 1, NULL, 0) == -1 ? errno : 0;
    client_ms = now_ms() - t0;
    ConnectDetach(coid);
    return arg;
}

/* A server that ignores unblock pulses and replies on SIGUSR1. */
static pid_t start_server(void)
{
    pid_t s = fork();
    if (s == 0) {
        *srv_chid = ChannelCreate(_NTO_CHF_UNBLOCK);
        sigset_t set;
        sigemptyset(&set);
        sigaddset(&set, SIGUSR1);
        sigprocmask(SIG_BLOCK, &set, NULL);
        char c;
        for (;;) {
            rcvid_t r = MsgReceive(*srv_chid, &c, 1, NULL);
            if (r == 0) {
                continue;
            }
            int sig;
            sigwait(&set, &sig);
            MsgReply(r, 0, NULL, 0);
        }
    }
    usleep(50000);
    return s;
}

/* One stuck client: the monitor's pulses within 400 ms (count, last
 * value). */
static int stuck(int mchid, uint64_t *value)
{
    pthread_t t;
    pthread_create(&t, NULL, client, NULL);
    struct _pulse p;
    struct sigevent to;
    SIGEV_UNBLOCK_INIT(&to);
    int n = 0;
    for (;;) {
        uint64_t ns = 400000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, &to, &ns, NULL);
        if (MsgReceivePulse(mchid, &p, sizeof p, NULL) == -1) {
            break;
        }
        if (p.code == 7) {
            n++;
            *value = (uint64_t)(uintptr_t)p.value.sival_ptr;
        }
    }
    kill(server, SIGUSR1);
    pthread_join(t, NULL);
    return n;
}

int main(void)
{
    srv_chid = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, -1, 0);
    check("without a monitor: ALIVE and IGNORE ESRCH",
          servmon(_PROC_SERVER_MONITOR_ALIVE, 0, 0, NULL) == -ESRCH &&
              servmon(_PROC_SERVER_MONITOR_IGNORE, 0, getppid(), NULL) == -ESRCH);
    pid_t d = fork();
    if (d == 0) {
        procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_SERVER_MONITOR, PROCMGR_AID_EOL);
        _exit(servmon(_PROC_SERVER_MONITOR_CONFIG, 100, 0, NULL) == -EPERM ? 0 : 1);
    }
    int st;
    waitpid(d, &st, 0);
    check("CONFIG takes SERVER_MONITOR", WIFEXITED(st) && WEXITSTATUS(st) == 0);

    int mchid = ChannelCreate(_NTO_CHF_PRIVATE);
    int mcoid = ConnectAttach(0, 0, mchid, _NTO_SIDE_CHANNEL, 0);
    struct sigevent ev;
    SIGEV_PULSE_PTR_INIT(&ev, mcoid, 10, 7, (void *)0xabcd00000000ULL);
    check("CONFIG, then ALIVE",
          servmon(_PROC_SERVER_MONITOR_CONFIG, 100, 0, &ev) == 0 && servmon(_PROC_SERVER_MONITOR_ALIVE, 0, 0, NULL) == 0);
    check("one monitor at most (EEXIST)", servmon(_PROC_SERVER_MONITOR_CONFIG, 100, 0, &ev) == -EEXIST);
    d = fork();
    if (d == 0) {
        _exit(servmon(_PROC_SERVER_MONITOR_IGNORE, 0, getppid(), NULL) == -EINVAL ? 0 : 1);
    }
    waitpid(d, &st, 0);
    check("IGNORE from another process EINVAL", WIFEXITED(st) && WEXITSTATUS(st) == 0);
    check("IGNORE of no process ESRCH", servmon(_PROC_SERVER_MONITOR_IGNORE, 0, 99999, NULL) == -ESRCH &&
                                            servmon(_PROC_SERVER_MONITOR_IGNORE, 0, 0, NULL) == -ESRCH);

    server = start_server();
    uint64_t value = 0;
    int n = stuck(mchid, &value);
    printf("servmontest:   %d pulse(s), value %#llx (pid %d tid %d), client %d after %llu ms\n", n,
           (unsigned long long)value, getpid(), client_tid, client_err, (unsigned long long)client_ms);
    check("a client still waiting: one event, the client's pid and tid in its value",
          n == 1 && value == ((uint64_t)(unsigned)getpid() << 32 | (unsigned)client_tid));
    check("the client waits for its server", client_err == 0 && client_ms > 300);
    check("IGNORE the server", servmon(_PROC_SERVER_MONITOR_IGNORE, 0, server, NULL) == 0);
    n = stuck(mchid, &value);
    printf("servmontest:   ignored: %d pulse(s), client %d after %llu ms\n", n, client_err,
           (unsigned long long)client_ms);
    check("an ignored server: no event, and it doesn't hold the client (ETIMEDOUT)",
          n == 0 && client_err == ETIMEDOUT && client_ms < 200);
    kill(server, SIGKILL);
    waitpid(server, NULL, 0);
    printf("servmontest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

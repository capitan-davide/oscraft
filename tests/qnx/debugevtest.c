/* A debugger's view of a process's end, through /proc/<pid>/as (as
 * procnto):
 * - a stop at a traced fault sends the debugger's (registered) event;
 * - SIGKILL to a process its debugger holds there doesn't end it, nor
 *   send the event: it stays stopped, not waitable, until the debugger
 *   closes it, and then dies of SIGKILL;
 * - a debugger starts with _DEBUG_FLAG_RLC; a process it stops is
 *   _NTO_PF_DEBUG_STOPPED;
 * - a debugged process that ends by itself (`exit(3)`) parks at its end:
 *   stopped at _DEBUG_WHY_TERMINATED (what: the status), its last thread
 *   still there, the process _NTO_PF_TERMING | _NTO_PF_DEBUG_STOPPED,
 *   not waitable, SIGKILL changing nothing; once the debugger closes it,
 *   its parent sees it killed by signal 0 (SIGCHLD's CLD_KILLED and
 *   status 0, a wait status of 0).
 * The children (`debugevtest crash`, `debugevtest exit`) write through a
 * null pointer or exit. Prints one "ok"/"FAIL" line per check and
 * "debugevtest: passed". */
#include <devctl.h>
#include <errno.h>
#include <fcntl.h>
#include <process.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/procfs.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static int chid, coid;
static volatile int nchld, chld_code = -1, chld_status = -1;

static void check(const char *what, int ok)
{
    printf("debugevtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void onchld(int signo, siginfo_t *si, void *ctx)
{
    nchld++;
    chld_code = si->si_code;
    chld_status = si->si_status;
}

/* The code of the next pulse within `ms`, else -errno. */
static int pulse(int ms)
{
    struct _pulse p;
    struct sigevent to;
    SIGEV_UNBLOCK_INIT(&to);
    uint64_t ns = (uint64_t)ms * 1000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, &to, &ns, NULL);
    int r = MsgReceive(chid, &p, sizeof p, NULL);
    return r == 0 ? p.code : -errno;
}

/* Spawn `self mode`, stop it, give it the debugger's event and run it
 * armed with `flags` (faults traced with _DEBUG_RUN_FAULT). Returns its
 * /proc/<pid>/as descriptor. */
static int debug(const char *self, const char *mode, pid_t *pid, unsigned flags)
{
    struct inheritance inh;
    memset(&inh, 0, sizeof inh);
    char *av[] = { (char *)self, (char *)mode, NULL };
    *pid = spawnp(self, 0, NULL, &inh, av, NULL);
    char path[64];
    snprintf(path, sizeof path, "/proc/%d/as", *pid);
    int fd = open(path, O_RDWR);
    procfs_status st;
    devctl(fd, DCMD_PROC_STOP, &st, sizeof st, NULL);
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, coid, 10, 7, 0);
    MsgRegisterEvent(&ev, fd);
    devctl(fd, DCMD_PROC_EVENT, &ev, sizeof ev, NULL);
    procfs_run run;
    memset(&run, 0, sizeof run);
    run.flags = _DEBUG_RUN_ARM | flags;
    run.fault = (1u << (FLTACCESS - 1)) | (1u << (FLTPAGE - 1)) | (1u << (FLTBOUNDS - 1));
    int r = devctl(fd, DCMD_PROC_RUN, &run, sizeof run, NULL);
    check("DCMD_PROC_RUN", r == 0);
    return fd;
}

static void fault_case(const char *self)
{
    pid_t pid;
    int fd = debug(self, "crash", &pid, _DEBUG_RUN_FAULT);
    check("the stop at the fault sends the event", pulse(2000) == 7);
    procfs_status st;
    memset(&st, 0, sizeof st);
    devctl(fd, DCMD_PROC_STATUS, &st, sizeof st, NULL);
    printf("debugevtest:   at the fault: flags %#x why %d what %d\n", st.flags, st.why, st.what);
    check("stopped at the fault (_DEBUG_WHY_FAULTED, FLTPAGE)",
          st.why == _DEBUG_WHY_FAULTED && st.what == FLTPAGE && (st.flags & _DEBUG_FLAG_ISTOP));
    check("_DEBUG_FLAG_RLC", (st.flags & _DEBUG_FLAG_MASK) == _DEBUG_FLAG_RLC);
    procfs_info info;
    memset(&info, 0, sizeof info);
    devctl(fd, DCMD_PROC_INFO, &info, sizeof info, NULL);
    printf("debugevtest:   at the fault: process flags %#x\n", info.flags);
    check("_NTO_PF_DEBUG_STOPPED", info.flags & _NTO_PF_DEBUG_STOPPED);

    kill(pid, SIGKILL);
    check("SIGKILL while held: no event", pulse(500) == -ETIMEDOUT);
    memset(&st, 0, sizeof st);
    int r = devctl(fd, DCMD_PROC_STATUS, &st, sizeof st, NULL);
    check("still stopped at the fault", r == 0 && st.why == _DEBUG_WHY_FAULTED && st.what == FLTPAGE);
    check("not waitable", waitpid(pid, NULL, WNOHANG) == 0);
    close(fd);
    int status = 0;
    int w = waitpid(pid, &status, 0);
    check("the debugger's close lets SIGKILL end it",
          w == pid && WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL);
    check("no event after the close", pulse(200) == -ETIMEDOUT);
}

static void exit_case(const char *self)
{
    pid_t pid;
    nchld = 0;
    int fd = debug(self, "exit", &pid, 0);
    check("its end sends the event", pulse(2000) == 7);
    procfs_status st;
    memset(&st, 0, sizeof st);
    int r = devctl(fd, DCMD_PROC_STATUS, &st, sizeof st, NULL);
    printf("debugevtest:   at its end: tid %d flags %#x why %d what %d\n", st.tid, st.flags, st.why, st.what);
    check("parked at its end (_DEBUG_WHY_TERMINATED, its status)",
          r == 0 && st.tid == 1 && st.why == _DEBUG_WHY_TERMINATED && st.what == 3
              && st.flags == (_DEBUG_FLAG_RLC | _DEBUG_FLAG_CURTID | _DEBUG_FLAG_ISTOP | _DEBUG_FLAG_STOPPED));
    procfs_info info;
    memset(&info, 0, sizeof info);
    r = devctl(fd, DCMD_PROC_INFO, &info, sizeof info, NULL);
    printf("debugevtest:   at its end: process flags %#x\n", info.flags);
    check("its last thread is there", r == 0 && info.num_threads == 1);
    check("_NTO_PF_TERMING | _NTO_PF_DEBUG_STOPPED",
          (info.flags & (_NTO_PF_TERMING | _NTO_PF_DEBUG_STOPPED | _NTO_PF_ZOMBIE))
              == (_NTO_PF_TERMING | _NTO_PF_DEBUG_STOPPED));
    check("not waitable, no SIGCHLD", waitpid(pid, NULL, WNOHANG) == 0 && nchld == 0);
    check("SIGKILL is accepted", kill(pid, SIGKILL) == 0);
    check("and changes nothing", pulse(300) == -ETIMEDOUT && waitpid(pid, NULL, WNOHANG) == 0);
    memset(&st, 0, sizeof st);
    devctl(fd, DCMD_PROC_STATUS, &st, sizeof st, NULL);
    check("still parked", st.why == _DEBUG_WHY_TERMINATED && (st.flags & _DEBUG_FLAG_ISTOP));
    close(fd);
    /* No wait held as it ends: a held wait would take its SIGCHLD. */
    for (int i = 0; i < 100 && nchld == 0; i++)
        usleep(10000);
    printf("debugevtest:   SIGCHLD: %d, si_code %d si_status %d\n", nchld, chld_code, chld_status);
    check("SIGCHLD: CLD_KILLED, status 0", nchld == 1 && chld_code == CLD_KILLED && chld_status == 0);
    int status = -1;
    int w = waitpid(pid, &status, 0);
    check("the debugger's close ends it, wait status 0", w == pid && status == 0);
}

int main(int argc, char **argv)
{
    if (argc > 1) {
        usleep(100000);
        if (strcmp(argv[1], "exit") == 0)
            exit(3);
        *(volatile int *)0 = 1;
        return 0;
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = onchld;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGCHLD, &sa, NULL);
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    fault_case(argv[0]);
    exit_case(argv[0]);

    printf("debugevtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

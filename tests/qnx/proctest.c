/* Process manager calls as procnto answers them: process groups and
 * sessions, resource limits, waits, kill(), priorities and scheduling,
 * sysconf() and confstr(), umask(), IDs, chdir()/chroot(), pathconf(),
 * supplementary groups (up to 384).
 * The expectations are procnto's results (measured in the reference
 * image). Prints one "ok"/"FAIL" line per call and "proctest: passed". */
#include <stdio.h>
#include <errno.h>
#include <unistd.h>
#include <signal.h>
#include <string.h>
#include <limits.h>
#include <sched.h>
#include <spawn.h>
#include <sys/wait.h>
#include <sys/resource.h>
#include <sys/utsname.h>
#include <sys/stat.h>
#include <sys/procmgr.h>
#include <grp.h>
static int fails;

/* `expr` fails with errno `err`. */
#define E(expr, err)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == -1 && errno == (err);                                                          \
        printf("proctest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);             \
        fails += !ok_;                                                                                 \
    } while (0)
/* `expr` gives `val`. */
#define V(expr, val)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == (long)(val);                                                                   \
        printf("proctest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);             \
        fails += !ok_;                                                                                 \
    } while (0)

int main(void)
{
    struct rlimit rl;
    siginfo_t si;
    char buf[256];
    E(getpgid(99999), ESRCH);
    E(getsid(99999), ESRCH);
    E(setpgid(99999, 0), ESRCH);
    E(setpgid(0, -1), EINVAL);
    E(setpgid(1, 1), ESRCH);
    E(getrlimit(99, &rl), EINVAL);
    E(setrlimit(99, &rl), EINVAL);
    V(getrlimit(RLIMIT_NOFILE, &rl), 0);
    V(rl.rlim_cur > 0, 1);
    rl.rlim_cur = rl.rlim_max + 1;
    E(setrlimit(RLIMIT_NOFILE, &rl), EINVAL);
    E(waitpid(99999, 0, 0), ECHILD);
    E(waitpid(-1, 0, WNOHANG), ECHILD);
    E(waitpid(0, 0, 0x8000000), EINVAL);
    E(waitid(P_PID, 99999, &si, WEXITED), ECHILD);
    E(waitid(99, 0, &si, WEXITED), ECHILD);
    E(waitid(P_ALL, 0, &si, 0), ECHILD);
    E(kill(99999, 0), ESRCH);
    E(kill(1, 0), EPERM);
    E(kill(1, SIGTERM), EPERM);
    E(kill(0, 99), EINVAL);
    E(killpg(99999, SIGTERM), ESRCH);
    V(getpriority(PRIO_PROCESS, 0), 0);
    V(getpriority(99, 0), 0);
    V(getpriority(PRIO_PROCESS, 99999), 0);
    V(setpriority(PRIO_PROCESS, 0, -30), 0);
    V(nice(0), 0);
    V(sched_get_priority_max(SCHED_FIFO), 253);
    V(sched_get_priority_min(SCHED_FIFO), 1);
    E(sched_get_priority_max(99), EINVAL);
    E(sched_getscheduler(99999), ESRCH);
    V(sched_getscheduler(0), 2);
    struct sched_param sp;
    sp.sched_priority = 10;
    E(sched_setscheduler(0, 99, &sp), EINVAL);
    E(sched_setscheduler(99999, SCHED_RR, &sp), ESRCH);
    V(sched_rr_get_interval(0, &(struct timespec){0}), 0);
    E(sched_rr_get_interval(99999, &(struct timespec){0}), ESRCH);
    V(sysconf(_SC_CHILD_MAX) > 0, 1);
    V(sysconf(_SC_OPEN_MAX), 1000);
    V(sysconf(_SC_PAGESIZE), 4096);
    V(sysconf(_SC_NPROCESSORS_ONLN), 4);
    V(sysconf(_SC_CLK_TCK), 100);
    V(sysconf(_SC_ARG_MAX), 61440);
    V(sysconf(_SC_NGROUPS_MAX), 384);
    V(sysconf(_SC_THREAD_STACK_MIN), 4096);
    V(sysconf(99999), -1);
    V(confstr(_CS_PATH, buf, sizeof buf) > 0, 1);
    V(confstr(99999, buf, sizeof buf), 0);
    V(confstr(_CS_HOSTNAME, buf, sizeof buf) > 0, 1);
    V(umask(022), 0);
    V(umask(0), 18);
    V(getuid(), 0);
    /* Supplementary groups: up to NGROUPS_MAX (384). */
    static gid_t groups[400], got[400];
    for (int i = 0; i < 400; i++)
        groups[i] = 100 + i;
    V(setgroups(384, groups), 0);
    V(getgroups(400, got) == 384 && got[383] == 483, 1);
    V(getgroups(0, got), 384);
    E(getgroups(5, got), EINVAL);
    E(setgroups(385, groups), EINVAL);
    V(setgroups(0, groups), 0);
    E(setuid(1234) == 0 ? (setuid(0)) : -2, EPERM);
    /* Unprivileged: the same groups again only; the count checked first. */
    V(setgroups(0, groups), 0);
    E(setgroups(1, groups), EPERM);
    E(setgroups(385, groups), EINVAL);
    E(setgid(99999), EPERM);
    E(seteuid(-2), EPERM);
    E(chdir("/nonexist"), ENOENT);
    E(chdir("/proc/boot/errtest"), ENOTDIR);
    E(chroot("/nonexist"), ENOENT);
    E(getcwd(buf, 1) == NULL ? -1 : 0, ERANGE);
    E(getcwd(buf, 0) == NULL ? -1 : 0, EINVAL);
    E(pathconf("/proc/boot", _PC_NAME_MAX), EINVAL);
    E(pathconf("/proc/boot", _PC_LINK_MAX), EINVAL);
    E(pathconf("/proc/boot", _PC_PATH_MAX), EINVAL);
    E(pathconf("/proc/boot", 9999), EINVAL);
    E(pathconf("/dev/null", _PC_NAME_MAX), EINVAL);
    E(fpathconf(0, _PC_PIPE_BUF), EINVAL);
    printf("proctest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

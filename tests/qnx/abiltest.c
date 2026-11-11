/* Abilities (procmgr_ability()): what a process may do, per domain (root,
 * non-root), as procnto applies them. Run as root. Each check runs in a
 * forked child, so that the abilities it drops don't affect the others;
 * `abiltest child <test>` is what a spawned child runs. Prints one
 * "ok"/"FAIL" line per check and "abiltest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <pthread.h>
#include <sched.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/mman.h>
#include <sys/mman_peer.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/procmsg.h>
#include <sys/resource.h>
#include <sys/sysmgr.h>
#include <sys/sysmsg.h>
#include <sys/trace.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;
static char *self;

static void check(const char *what, int ok)
{
    printf("abiltest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Set the clock to what it is (needs PROCMGR_AID_CLOCKSET). */
static int clockset(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return clock_settime(CLOCK_REALTIME, &ts) == 0 ? 0 : errno;
}

/* Run `fn` in a forked child; its exit status. */
static int in_child(int (*fn)(void))
{
    pid_t pid = fork();
    if (pid == 0) {
        _exit(fn());
    }
    int st;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : 255;
}

/* Spawn `abiltest child <what>`; its exit status. */
static int spawned(const char *what)
{
    char *argv[] = { self, "child", (char *)what, NULL };
    pid_t pid;
    if (posix_spawnp(&pid, self, NULL, NULL, argv, NULL) != 0) {
        return 254;
    }
    int st;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : 255;
}

static int deny_clockset(void)
{
    if (procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL) != EOK) {
        return 100;
    }
    return clockset();
}

static int deny_then_allow(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    if (procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_ALLOW | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL) != EOK) {
        return 100;
    }
    return clockset();
}

static int locked(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AOP_LOCK | PROCMGR_AID_CLOCKSET,
                    PROCMGR_AID_EOL);
    int r = procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_ALLOW | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    int c = clockset();
    printf("abiltest:   allow after lock = %d (%s), then clock_settime = %d\n", r, strerror(r), c);
    return c == 0 ? 0 : 1;
}

static int deny_interrupt(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_INTERRUPT, PROCMGR_AID_EOL);
    int id = InterruptAttachThread(200, _NTO_INTR_FLAGS_NO_UNMASK);
    return id == -1 ? errno : 0;
}

static int deny_setuid(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_SETUID, PROCMGR_AID_EOL);
    return setuid(1000) == 0 ? 0 : errno;
}

static int setuid_subrange(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_SUBRANGE | PROCMGR_AID_SETUID, (uint64_t)2000, (uint64_t)3000,
                    PROCMGR_AID_EOL);
    int a = setuid(1000) == 0 ? 0 : errno;
    int b = setuid(2500) == 0 ? 0 : errno;
    printf("abiltest:   SETUID subrange 2000-3000: setuid(1000) = %d, setuid(2500) = %d\n", a, b);
    return a == EPERM && b == 0 ? 0 : 1;
}

static void *nothing(void *arg)
{
    return arg;
}

static int create_at(int prio)
{
    pthread_attr_t a;
    struct sched_param sp = { .sched_priority = prio };
    pthread_attr_init(&a);
    pthread_attr_setinheritsched(&a, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&a, SCHED_RR);
    pthread_attr_setschedparam(&a, &sp);
    pthread_t t;
    int r = pthread_create(&t, &a, nothing, NULL);
    if (r == 0) {
        pthread_join(t, NULL);
    }
    return r;
}

static int deny_priority(void)
{
    int max = sched_get_priority_max(SCHED_RR);
    int p253 = pthread_setschedprio(pthread_self(), 253);
    int p254 = pthread_setschedprio(pthread_self(), 254);
    int t254 = create_at(254);
    pthread_setschedprio(pthread_self(), 10);
    printf("abiltest:   root: max %d; priority 253 = %d, 254 = %d, a thread at 254 = %d\n", max, p253, p254, t254);
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_PRIORITY, PROCMGR_AID_EOL);
    int a = pthread_setschedprio(pthread_self(), 30);
    int b = pthread_setschedprio(pthread_self(), 80);
    int c = create_at(80);
    int dmax = sched_get_priority_max(SCHED_RR);
    printf("abiltest:   PRIORITY denied: priority 30 = %d, 80 = %d, a thread at 80 = %d, max %d\n", a, b, c, dmax);
    setuid(1000);
    int nmax = sched_get_priority_max(SCHED_RR);
    int n80 = create_at(80);
    printf("abiltest:   non-root: max %d, a thread at 80 = %d\n", nmax, n80);
    return max == 253 && p253 == 0 && p254 == EINVAL && t254 == EINVAL && a == 0 && b == EPERM && c == EPERM
                   && dmax == 63 && nmax == 63 && n80 == EPERM
               ? 0
               : 1;
}

static int deny_mem_phys(void)
{
    void *a = mmap_device_memory(NULL, 4096, PROT_READ | PROT_NOCACHE, 0, 0xfe215000);
    int ok = a != MAP_FAILED;
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_MEM_PHYS, PROCMGR_AID_EOL);
    errno = 0;
    void *b = mmap_device_memory(NULL, 4096, PROT_READ | PROT_NOCACHE, 0, 0xfe215000);
    int e = errno;
    printf("abiltest:   mmap_device_memory: allowed %d, MEM_PHYS denied: %s (%d)\n", ok, b == MAP_FAILED ? "fails" : "works", e);
    return ok && b == MAP_FAILED && e == EPERM ? 0 : 1;
}

static int deny_io(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_IO, PROCMGR_AID_EOL);
    return ThreadCtl(_NTO_TCTL_IO, 0) == -1 ? errno : 0;
}

static int deny_all(void)
{
    if (procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_EOL) != EOK) {
        return 100;
    }
    int c = clockset();
    int s = setuid(1000) == 0 ? 0 : errno;
    printf("abiltest:   denied all: clock_settime = %d, setuid = %d\n", c, s);
    return c == EPERM && s == EPERM ? 0 : 1;
}

static int nonroot_grant(void)
{
    if (setuid(1000) != 0) {
        return 100;
    }
    int r = procmgr_ability(0, PROCMGR_ADN_NONROOT | PROCMGR_AOP_ALLOW | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    int c = clockset();
    printf("abiltest:   non-root grants itself CLOCKSET = %d (%s), clock_settime = %d\n", r, strerror(r), c);
    return r == EPERM && c == EPERM ? 0 : 1;
}

static int nonroot_allowed(void)
{
    /* As root, allow CLOCKSET to the non-root domain, then become non-root. */
    procmgr_ability(0, PROCMGR_ADN_NONROOT | PROCMGR_AOP_ALLOW | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    if (setuid(1000) != 0) {
        return 100;
    }
    return clockset();
}

static int spawn_no_inherit(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    return spawned("clockset");
}

static int spawn_inherit(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AOP_INHERIT_YES | PROCMGR_AID_CLOCKSET,
                    PROCMGR_AID_EOL);
    return spawned("clockset");
}

static int fork_copies(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
    return in_child(clockset);
}

/* Operations that need an ability, each run allowed and then denied
 * (as procnto: EPERM). */
static pid_t other; /* a process of user 1000 */
static volatile int *other_chid; /* a channel it made */

static int r_errno(int ok)
{
    return ok ? 0 : errno;
}
static int o_signal(void) { return r_errno(kill(other, 0) == 0); }
static int o_rlimit(void)
{
    struct rlimit r;
    getrlimit(RLIMIT_NOFILE, &r);
    r.rlim_max += 10;
    r.rlim_cur = r.rlim_max;
    return r_errno(setrlimit(RLIMIT_NOFILE, &r) == 0);
}
static int o_pgrp(void) { return r_errno(setpgid(0, 0) == 0); }
static int o_chroot(void) { return r_errno(chroot("/") == 0); }
static int o_map_fixed(void)
{
    void *a = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    return r_errno(mmap(a, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, NOFD, 0) != MAP_FAILED);
}
static int o_prot_wx(void)
{
    return r_errno(mmap(NULL, 4096, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_PRIVATE | MAP_ANON, NOFD, 0)
                   != MAP_FAILED);
}
static int o_mem_lock(void)
{
    static char buf[4096];
    return r_errno(mlock(buf, sizeof buf) == 0);
}
static int o_fork(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        _exit(0);
    }
    if (pid == -1) {
        return errno;
    }
    waitpid(pid, NULL, 0);
    return 0;
}
static int o_public_channel(void) { return r_errno(ChannelCreate(0) != -1); }
static int spawn_flags(unsigned xflags)
{
    posix_spawnattr_t a;
    posix_spawnattr_init(&a);
    if (xflags) {
        posix_spawnattr_setxflags(&a, xflags);
    }
    char *argv[] = { self, "child", "none", NULL };
    pid_t pid;
    int e = posix_spawnp(&pid, self, NULL, &a, argv, NULL);
    if (e == 0) {
        waitpid(pid, NULL, 0);
    }
    return e;
}
static int o_spawn(void) { return spawn_flags(0); }
static int o_xquery(void)
{
    clockid_t c;
    struct timespec ts;
    int e = clock_getcpuclockid(other, &c);
    return e ? e : r_errno(clock_gettime(c, &ts) == 0);
}
static int o_connection(void)
{
    int coid = ConnectAttach(0, other, *other_chid, 0, 0);
    if (coid == -1) {
        return errno;
    }
    int r = r_errno(MsgSendPulse(coid, 10, 1, 0) == 0);
    ConnectDetach(coid);
    return r;
}
static int o_session(void) { return r_errno(procmgr_session(0, getsid(0), 0, PROCMGR_SESSION_SIGNAL_LEADER) == 0); }
static int o_newapp(void) { return spawn_flags(POSIX_SPAWN_NEWAPP); }
static int o_mem_special(void)
{
    int fd = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    int r = r_errno(shm_ctl_special(fd, SHMCTL_ANON, 0, 4096, 0) == 0);
    close(fd);
    return r;
}
static int o_srandom(void)
{
    uint64_t seed = 1, entropy[2];
    return r_errno(SysSrandom(&seed, entropy, 2) == 0);
}

static int o_hrt(void)
{
    timer_t t;
    timer_create(CLOCK_MONOTONIC, NULL, &t);
    struct itimerspec tol;
    memset(&tol, 0, sizeof tol);
    tol.it_value.tv_nsec = 1;
    return r_errno(timer_settime(t, TIMER_TOLERANCE, &tol, NULL) == 0);
}
static int o_pathspace(void)
{
    static resmgr_connect_funcs_t cf;
    static resmgr_io_funcs_t iof;
    static iofunc_attr_t attr;
    resmgr_attr_t ra;
    memset(&ra, 0, sizeof ra);
    iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &cf, _RESMGR_IO_NFUNCS, &iof);
    iofunc_attr_init(&attr, S_IFNAM | 0666, NULL, NULL);
    return r_errno(resmgr_attach(dispatch_create(), &ra, "/dev/abiltest", _FTYPE_ANY, 0, &cf, &iof, &attr) != -1);
}
static int o_trace(void) { return r_errno(TraceEvent(_NTO_TRACE_STOP) != -1); }
static int o_mem_peer(void)
{
    return r_errno(mmap_peer(other, NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0)
                   != MAP_FAILED);
}

/* _PROC_UMASK as libc sends it, with a pid (umask() sends 0). */
static int proc_umask(int subtype, unsigned mask, pid_t pid, unsigned *old)
{
    proc_umask_t m;
    memset(&m, 0, sizeof m);
    m.i.type = _PROC_UMASK;
    m.i.subtype = subtype;
    m.i.umask = mask;
    m.i.pid = pid;
    if (MsgSend(PROCMGR_COID, &m.i, sizeof m.i, &m.o, sizeof m.o) == -1) {
        return errno;
    }
    if (old) {
        *old = m.o.umask;
    }
    return 0;
}
static int o_umask(void) { return proc_umask(_PROC_UMASK_SET, 022, other, NULL); }
static int o_cpumode(void) { return r_errno(sysmgr_cpumode(0) == 0); }

/* MsgQueueOpen() as a queue manager, for a message of our own. */
static int mq_coid;
static void *mq_client(void *arg)
{
    char c = 0;
    MsgSend(mq_coid, &c, 1, NULL, 0);
    return arg;
}
static int queue_open(int queueid)
{
    int chid = ChannelCreate(_NTO_CHF_PRIVATE);
    mq_coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    pthread_t t;
    pthread_create(&t, NULL, mq_client, NULL);
    char c;
    rcvid_t rcvid = MsgReceive(chid, &c, 1, NULL);
    int r = r_errno(MsgQueueOpen(rcvid, queueid, 16, 4, _IO_FLAG_RD | _IO_FLAG_WR) != -1);
    MsgReply(rcvid, 0, NULL, 0);
    pthread_join(t, NULL);
    ConnectDetach(mq_coid);
    ChannelDestroy(chid);
    return r;
}
static int o_msg_queue(void) { return queue_open(-1); }

static const struct {
    const char *name;
    unsigned aid;
    int (*fn)(void);
} needs[] = {
    { "HIGH_RESOLUTION_TIMER: TIMER_TOLERANCE below the tick", PROCMGR_AID_HIGH_RESOLUTION_TIMER, o_hrt },
    { "PATHSPACE: resmgr_attach()", PROCMGR_AID_PATHSPACE, o_pathspace },
    { "TRACE: controlling tracing", PROCMGR_AID_TRACE, o_trace },
    { "MEM_PEER: mmap_peer()", PROCMGR_AID_MEM_PEER, o_mem_peer },
    { "SIGNAL: kill() of another user's process", PROCMGR_AID_SIGNAL, o_signal },
    { "RLIMIT: raising a hard limit", PROCMGR_AID_RLIMIT, o_rlimit },
    { "PGRP: setpgid()", PROCMGR_AID_PGRP, o_pgrp },
    { "CHROOT: chroot()", PROCMGR_AID_CHROOT, o_chroot },
    { "MAP_FIXED: mmap(MAP_FIXED)", PROCMGR_AID_MAP_FIXED, o_map_fixed },
    { "PROT_WRITE_AND_EXEC: writable and executable memory", PROCMGR_AID_PROT_WRITE_AND_EXEC, o_prot_wx },
    { "MEM_LOCK: mlock()", PROCMGR_AID_MEM_LOCK, o_mem_lock },
    { "FORK: fork()", PROCMGR_AID_FORK, o_fork },
    { "PUBLIC_CHANNEL: a channel without _NTO_CHF_PRIVATE", PROCMGR_AID_PUBLIC_CHANNEL, o_public_channel },
    { "SRANDOM: SysSrandom() taking entropy", PROCMGR_AID_SRANDOM, o_srandom },
    { "SPAWN: posix_spawn()", PROCMGR_AID_SPAWN, o_spawn },
    { "CHILD_NEWAPP: posix_spawn(POSIX_SPAWN_NEWAPP)", PROCMGR_AID_CHILD_NEWAPP, o_newapp },
    { "MEM_SPECIAL: shm_ctl_special()", PROCMGR_AID_MEM_SPECIAL, o_mem_special },
    { "XPROCESS_QUERY: another user's CPU-time clock", PROCMGR_AID_XPROCESS_QUERY, o_xquery },
    { "CONNECTION: a pulse to another user's channel", PROCMGR_AID_CONNECTION, o_connection },
    { "SESSION: procmgr_session() signals", PROCMGR_AID_SESSION, o_session },
    { "UMASK: another user's umask", PROCMGR_AID_UMASK, o_umask },
    { "CPUMODE: sysmgr_cpumode()", PROCMGR_AID_CPUMODE, o_cpumode },
    { "MSG_QUEUE: MsgQueueOpen() making a queue", PROCMGR_AID_MSG_QUEUE, o_msg_queue },
};

/* As procnto: a process of user 1000 sets the umask of processes whose
 * user IDs are all 1000, reads anyone's, and the value is kept as given;
 * another subtype is ENOSYS. */
static int umask_rules(void)
{
    pid_t same = fork();
    if (same == 0) {
        setuid(1000);
        sleep(10);
        _exit(0);
    }
    pid_t saved = fork(); /* real and effective 1000, saved 0 */
    if (saved == 0) {
        setreuid(1000, -1);
        seteuid(1000);
        sleep(10);
        _exit(0);
    }
    usleep(50000);
    setuid(1000);
    unsigned old = 0;
    int bad = proc_umask(2, 0, 0, NULL);
    int get_root = proc_umask(_PROC_UMASK_GET, 0, 1, &old);
    int set_same = proc_umask(_PROC_UMASK_SET, 0xfffff, same, NULL);
    int get_same = proc_umask(_PROC_UMASK_GET, 0, same, &old);
    int set_saved = proc_umask(_PROC_UMASK_SET, 07, saved, NULL);
    int qbad = r_errno(MsgQueueOpen(0, -5, 16, 4, 0) != -1);
    kill(same, SIGKILL);
    kill(saved, SIGKILL);
    printf("abiltest:   umask: subtype 2 %d, get root's %d, set same user's %d (now %x), saved ID 0 %d;"
           " MsgQueueOpen(-5) %d\n", bad, get_root, set_same, get_same ? 0 : old, set_saved, qbad);
    return bad == ENOSYS && get_root == 0 && set_same == 0 && old == 0xfffff && set_saved == EPERM
                   && qbad == EINVAL
               ? 0
               : 1;
}

static unsigned need_index;

static int need_denied(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | needs[need_index].aid, PROCMGR_AID_EOL);
    return needs[need_index].fn();
}

static int need_allowed(void)
{
    return needs[need_index].fn();
}

/* REBOOT, denied only (allowed, it would reboot). */
static int deny_reboot(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_REBOOT, PROCMGR_AID_EOL);
    return sysmgr_reboot() == 0 ? 0 : errno;
}

/* Non-root, POSIX: signals to one's own processes only. */
static int nonroot_signal(void)
{
    pid_t mine = fork();
    if (mine == 0) {
        setuid(1000);
        sleep(10);
        _exit(0);
    }
    usleep(50000);
    setuid(1000);
    int own = kill(mine, 0) == 0 ? 0 : errno;
    int root = kill(1, 0) == 0 ? 0 : errno; /* procnto's */
    kill(mine, SIGKILL);
    printf("abiltest:   non-root: kill() of its user's process %d, of root's %d\n", own, root);
    return own == 0 && root == EPERM ? 0 : 1;
}

int main(int argc, char **argv)
{
    self = argv[0];
    if (argc == 3 && !strcmp(argv[1], "child")) {
        return !strcmp(argv[2], "clockset") ? clockset() : !strcmp(argv[2], "none") ? 0 : 99;
    }
    int r;
    r = in_child(clockset);
    check("root may set the clock", r == 0);
    r = in_child(deny_clockset);
    printf("abiltest:   deny CLOCKSET: clock_settime = %d\n", r);
    check("CLOCKSET denied: clock_settime EPERM", r == EPERM);
    r = in_child(deny_then_allow);
    check("CLOCKSET denied, then allowed again (root has ABLE_PRIV)", r == 0);
    r = in_child(locked);
    check("a locked denial stays", r == 1);
    r = in_child(deny_interrupt);
    printf("abiltest:   deny INTERRUPT: InterruptAttachThread = %d\n", r);
    check("INTERRUPT denied: InterruptAttachThread EPERM", r == EPERM);
    r = in_child(deny_setuid);
    printf("abiltest:   deny SETUID: setuid = %d\n", r);
    check("SETUID denied: setuid EPERM", r == EPERM);
    check("a SETUID subrange limits the IDs", in_child(setuid_subrange) == 0);
    check("priorities: 1-253 for root, 1-63 without PRIORITY (non-root too)", in_child(deny_priority) == 0);
    check("MEM_PHYS denied: mmap_device_memory() EPERM", in_child(deny_mem_phys) == 0);
    check("IO denied: ThreadCtl(_NTO_TCTL_IO) EPERM", in_child(deny_io) == EPERM);
    check("everything denied (PROCMGR_AID_EOL with an operation)", in_child(deny_all) == 0);
    check("non-root can't grant itself a privileged ability", in_child(nonroot_grant) == 0);
    r = in_child(nonroot_allowed);
    printf("abiltest:   CLOCKSET allowed to non-root: clock_settime = %d\n", r);
    check("an ability allowed for the non-root domain", r == 0);
    r = in_child(spawn_no_inherit);
    printf("abiltest:   spawned child, denial not inherited: clock_settime = %d\n", r);
    check("a denial isn't inherited by spawn() by default", r == 0);
    r = in_child(spawn_inherit);
    printf("abiltest:   spawned child, denial inherited: clock_settime = %d\n", r);
    check("PROCMGR_AOP_INHERIT_YES: inherited by spawn()", r == EPERM);
    other_chid = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, -1, 0);
    other = fork();
    if (other == 0) {
        setuid(1000);
        *other_chid = ChannelCreate(0);
        sleep(60);
        _exit(0);
    }
    usleep(100000);
    for (need_index = 0; need_index < sizeof needs / sizeof needs[0]; need_index++) {
        int a = in_child(need_allowed), d = in_child(need_denied);
        if (a != 0 || d != EPERM) {
            printf("abiltest:   %s: allowed %d, denied %d\n", needs[need_index].name, a, d);
        }
        check(needs[need_index].name, a == 0 && d == EPERM);
    }
    kill(other, SIGKILL);
    waitpid(other, NULL, 0);
    check("non-root signals its own processes only", in_child(nonroot_signal) == 0);
    check("umask of another process, as procnto; MsgQueueOpen() queue ID -5 EINVAL", in_child(umask_rules) == 0);
    check("REBOOT denied: sysmgr_reboot() EPERM", in_child(deny_reboot) == EPERM);
    r = in_child(fork_copies);
    printf("abiltest:   forked child: clock_settime = %d\n", r);
    check("fork() copies the abilities", r == EPERM);
    printf("abiltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

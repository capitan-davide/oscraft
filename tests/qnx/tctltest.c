/* ThreadCtl() commands as procnto answers them:
 * - _NTO_TCTL_THREADS_HOLD/_CONT and _ONE_THREAD_HOLD/_CONT: a held
 *   thread stops (STATE_STOPPED), running or sleeping; holds don't count;
 *   either continue undoes either hold, SIGCONT doesn't; holding oneself
 *   or thread 0 does nothing, no such thread is ESRCH; threads created
 *   after a hold run;
 * - _NTO_TCTL_ALIGN_FAULT: the old setting (initially -1) comes back as
 *   1 or -1; with 1 a misaligned access is SIGBUS (BUS_ADRALN, fault 5)
 *   (libc itself isn't written for that: pthread_create() faults);
 * - _NTO_TCTL_ADD_EXIT_EVENT/_DEL_EXIT_EVENT: the event when the thread
 *   ends, none once deleted;
 * - _NTO_TCTL_HYP_VCPU: 0; unknown commands: EINVAL;
 * - _NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT: a size of 1 or 2 words works
 *   (the old masks come back in the first word of each array, the
 *   inherit mask as every CPU); 0, negative, 3 or more (a kernel
 *   allocation sized by the user, review item SCHED-1): EINVAL.
 * - runmasks of another thread of the process (measured, review item
 *   SCHED-10): read (a new mask of 0), never set: ENOTSUP, also for a
 *   thread there isn't; _NTO_TCTL_RUNMASK with 0: EINVAL.
 * Prints one "ok"/"FAIL" line per check and "tctltest: passed". */
#include <devctl.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdint.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/procfs.h>
#include <sys/syspage.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static volatile unsigned long counter;
static volatile int stop;
static int chid;

static void check(const char *what, int ok)
{
    printf("tctltest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *spinner(void *arg)
{
    (void)arg;
    while (!stop) {
        counter++;
    }
    return NULL;
}

static void *sleeper(void *arg)
{
    (void)arg;
    while (!stop) {
        counter++;
        usleep(1000);
    }
    return NULL;
}

/* Whether `counter` moves within 50 ms. */
static int moves(void)
{
    unsigned long c = counter;
    usleep(50000);
    return counter != c;
}

/* Thread `tid`'s state (DCMD_PROC_TIDSTATUS), -1 if unknown. */
static int state_of(int tid)
{
    int fd = open("/proc/self/as", O_RDONLY);
    procfs_status st;
    memset(&st, 0, sizeof st);
    st.tid = tid;
    int r = devctl(fd, DCMD_PROC_TIDSTATUS, &st, sizeof st, NULL);
    close(fd);
    return r == EOK && st.tid == tid ? st.state : -1;
}

static void stop_thread(pthread_t t)
{
    stop = 1;
    pthread_join(t, NULL);
    stop = 0;
}

static void holds(void)
{
    pthread_t t;
    pthread_create(&t, NULL, spinner, NULL);
    int tid = (int)t;
    usleep(20000);
    check("THREADS_HOLD", ThreadCtl(_NTO_TCTL_THREADS_HOLD, 0) == 0);
    check("the other thread stops", !moves() && state_of(tid) == STATE_STOPPED);
    ThreadCtl(_NTO_TCTL_THREADS_HOLD, 0);
    check("THREADS_CONT", ThreadCtl(_NTO_TCTL_THREADS_CONT, 0) == 0);
    check("one continue after two holds: it runs", moves() && state_of(tid) != STATE_STOPPED);
    check("ONE_THREAD_HOLD", ThreadCtl(_NTO_TCTL_ONE_THREAD_HOLD, (void *)(long)tid) == 0);
    check("it stops", !moves() && state_of(tid) == STATE_STOPPED);
    kill(getpid(), SIGCONT);
    check("SIGCONT doesn't continue it", !moves());
    ThreadCtl(_NTO_TCTL_THREADS_CONT, 0);
    check("THREADS_CONT continues it", moves());
    check("ONE_THREAD_CONT of a running thread", ThreadCtl(_NTO_TCTL_ONE_THREAD_CONT, (void *)(long)tid) == 0);
    check("holding oneself does nothing", ThreadCtl(_NTO_TCTL_ONE_THREAD_HOLD, (void *)1L) == 0);
    check("nor thread 0", ThreadCtl(_NTO_TCTL_ONE_THREAD_HOLD, (void *)0L) == 0);
    errno = 0;
    check("no such thread: ESRCH", ThreadCtl(_NTO_TCTL_ONE_THREAD_HOLD, (void *)99L) == -1 && errno == ESRCH);
    errno = 0;
    check("continuing none: ESRCH", ThreadCtl(_NTO_TCTL_ONE_THREAD_CONT, (void *)99L) == -1 && errno == ESRCH);
    stop_thread(t);

    pthread_create(&t, NULL, sleeper, NULL);
    tid = (int)t;
    usleep(10000);
    ThreadCtl(_NTO_TCTL_ONE_THREAD_HOLD, (void *)(long)tid);
    check("a sleeping thread held stops", !moves() && state_of(tid) == STATE_STOPPED);
    ThreadCtl(_NTO_TCTL_ONE_THREAD_CONT, (void *)(long)tid);
    check("and goes on", moves());
    stop_thread(t);

    ThreadCtl(_NTO_TCTL_THREADS_HOLD, 0);
    pthread_create(&t, NULL, spinner, NULL);
    check("a thread created after THREADS_HOLD runs", moves());
    ThreadCtl(_NTO_TCTL_THREADS_CONT, 0);
    stop_thread(t);
}

static sigjmp_buf env;
static volatile siginfo_t caught;

static void on_bus(int signo, siginfo_t *si, void *ctx)
{
    (void)signo;
    (void)ctx;
    int v = -1;
    ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    caught = *si;
    siglongjmp(env, 1);
}

static void alignment(void)
{
    int v = 0, r[4], old[4];
    r[0] = ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    old[0] = v;
    v = 1;
    r[1] = ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    old[1] = v;
    v = 0;
    r[2] = ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    old[2] = v;
    v = -5;
    r[3] = ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    old[3] = v;
    check("ALIGN_FAULT: -1 at first, then the old setting",
          r[0] == 0 && r[1] == 0 && r[2] == 0 && r[3] == 0 && old[0] == -1 && old[1] == -1 && old[2] == 1 &&
              old[3] == 1);
    v = 0;
    ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    check("a negative setting reads -1", v == -1);
    errno = 0;
    check("a null pointer: EFAULT", ThreadCtl(_NTO_TCTL_ALIGN_FAULT, NULL) == -1 && errno == EFAULT);

    static char buf[16] __attribute__((aligned(16)));
    volatile long *p = (volatile long *)(buf + 1);
    *p = 0x1122334455667788L;
    check("misaligned accesses work at -1", *p == 0x1122334455667788L);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_bus;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGBUS, &sa, NULL);
    long x = 0;
    if (sigsetjmp(env, 1) == 0) {
        v = 1;
        ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
        x = *p;
        v = -1;
        ThreadCtl(_NTO_TCTL_ALIGN_FAULT, &v);
    }
    check("at 1 a misaligned load is SIGBUS, BUS_ADRALN, fault 5",
          x == 0 && caught.si_signo == SIGBUS && caught.si_code == BUS_ADRALN && caught.si_fltno == 5 &&
              caught.si_addr == (void *)p);
    signal(SIGBUS, SIG_DFL);
}

static void *exiter(void *arg)
{
    long how = (long)arg;
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0), 10, 5, how);
    if (how != 3) {
        ThreadCtl(_NTO_TCTL_ADD_EXIT_EVENT, &ev);
    }
    if (how == 2) {
        ThreadCtl(_NTO_TCTL_DEL_EXIT_EVENT, 0);
    }
    return NULL;
}

/* The next pulse's value within 100 ms (-1 if none). */
static int next_pulse(void)
{
    struct _pulse p;
    memset(&p, 0, sizeof p);
    uint64_t ns = 100000000;
    TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &ns, NULL);
    if (MsgReceive(chid, &p, sizeof p, NULL) != 0 || p.code != 5) {
        return -1;
    }
    return p.value.sival_int;
}

static void exit_events(void)
{
    chid = ChannelCreate(_NTO_CHF_PRIVATE);
    pthread_t t;
    pthread_create(&t, NULL, exiter, (void *)1);
    pthread_join(t, NULL);
    check("ADD_EXIT_EVENT: the event when the thread ends", next_pulse() == 1);
    pthread_create(&t, NULL, exiter, (void *)2);
    pthread_join(t, NULL);
    check("DEL_EXIT_EVENT: none", next_pulse() == -1);
    check("DEL_EXIT_EVENT with none", ThreadCtl(_NTO_TCTL_DEL_EXIT_EVENT, 0) == 0);
    errno = 0;
    check("ADD_EXIT_EVENT of a null pointer: EFAULT", ThreadCtl(_NTO_TCTL_ADD_EXIT_EVENT, NULL) == -1 && errno == EFAULT);
}

static int runmask_inherit(int size, int nwords, unsigned *out)
{
    /* size, then runmask[nwords], inherit_mask[nwords]: all CPUs */
    unsigned buf[2 + 2 * 64];
    memset(buf, 0, sizeof buf);
    buf[0] = (unsigned)size;
    for (int i = 0; i < nwords; i++)
        buf[1 + i] = buf[1 + nwords + i] = 0xffffffffu;
    errno = 0;
    int r = ThreadCtl(_NTO_TCTL_RUNMASK_GET_AND_SET_INHERIT, buf);
    memcpy(out, buf + 1, 4 * sizeof *out);
    return r;
}

static void runmask_sizes(void)
{
    unsigned w[4];
    unsigned all = (1u << _syspage_ptr->num_cpu) - 1;
    check("RUNMASK_GET_AND_SET_INHERIT, 1 word", runmask_inherit(1, 1, w) == 0 && w[0] == all && w[1] == all);
    check("2 words: old masks in the first word of each", runmask_inherit(2, 2, w) == 0 && w[0] == all && w[1] == 0 && w[2] == all && w[3] == 0);
    check("3 words: EINVAL", runmask_inherit(3, 3, w) == -1 && errno == EINVAL);
    check("64 words: EINVAL", runmask_inherit(64, 64, w) == -1 && errno == EINVAL);
    check("size 0: EINVAL", runmask_inherit(0, 1, w) == -1 && errno == EINVAL);
    check("size -1: EINVAL", runmask_inherit(-1, 1, w) == -1 && errno == EINVAL);
    check("size 0x7fffffff: EINVAL, kernel intact", runmask_inherit(0x7fffffff, 64, w) == -1 && errno == EINVAL);
    check("1 word again", runmask_inherit(1, 1, w) == 0 && w[0] == all);
}

static void other_runmask(void)
{
    pthread_t t;
    pthread_create(&t, NULL, sleeper, NULL);
    usleep(10000);
    unsigned m = 0;
    uint64_t m64 = 0;
    errno = 0;
    check("RUNMASK of 0: EINVAL", ThreadCtl(_NTO_TCTL_RUNMASK, 0) == -1 && errno == EINVAL);
    check("another thread's runmask read (GET_AND_SET 0)",
          ThreadCtlExt(0, t, _NTO_TCTL_RUNMASK_GET_AND_SET, &m) == 0 && m != 0);
    check("and read 64 bits wide", ThreadCtlExt(0, t, 7, &m64) == 0 && m64 != 0);
    errno = 0;
    check("set: ENOTSUP", ThreadCtlExt(0, t, _NTO_TCTL_RUNMASK, (void *)1L) == -1 && errno == ENOTSUP);
    m = 1;
    errno = 0;
    check("GET_AND_SET of a mask: ENOTSUP", ThreadCtlExt(0, t, _NTO_TCTL_RUNMASK_GET_AND_SET, &m) == -1 && errno == ENOTSUP);
    errno = 0;
    check("a thread there isn't: ENOTSUP too", ThreadCtlExt(0, 9999, _NTO_TCTL_RUNMASK, (void *)1L) == -1 && errno == ENOTSUP);
    stop_thread(t);
}

int main(void)
{
    other_runmask();
    holds();
    alignment();
    exit_events();
    runmask_sizes();
    check("HYP_VCPU: 0", ThreadCtl(_NTO_TCTL_HYP_VCPU, 0) == 0);
    errno = 0;
    check("an unknown command: EINVAL", ThreadCtl(12, 0) == -1 && errno == EINVAL);
    printf("tctltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

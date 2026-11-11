/* Trace buffers (_SYS_TRACE ALLOCBUF, DEALLOCBUF, DETACH) as procnto
 * answers them (measured), and who may see them (review IPC-20, #83):
 * - only the logger (_NTO_TRACE_LOGGER_ATTACH) allocates or frees them
 *   (EPERM); none: EINVAL; past 32 bits of buffers: EOVERFLOW;
 * - buffers are never replaced: a second ALLOCBUF is EEXIST;
 * - the logger's view isn't inherited by a fork (MAP_NOINHERIT, as any
 *   MAP_NOINHERIT mapping);
 * - DEALLOCBUF is EBUSY while tracing, and after a stop until the logger
 *   has taken each CPU's buffers (WAITBUFFER to the end); then the view
 *   is gone;
 * - the logger's DETACH, or its end, frees them; another's does nothing.
 * Prints one "ok"/"FAIL" line per check and "traceviewtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/sysmsg.h>
#include <sys/trace.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("traceviewtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static uintptr_t got;

/* _SYS_TRACE subtype `sub` (`per` buffers per CPU): -errno or 0, the
 * address in `got`. */
static int sys_trace(unsigned sub, unsigned per)
{
    unsigned char m[8] = {_SYS_TRACE & 0xff, _SYS_TRACE >> 8, sub, 0};
    memcpy(m + 4, &per, 4);
    uint64_t base = 0;
    long r = MsgSend(SYSMGR_COID, m, sizeof m, &base, sizeof base);
    got = base;
    return r == -1 ? -errno : 0;
}

static sigjmp_buf jb;

static void on_sig(int signo)
{
    siglongjmp(jb, signo);
}

/* Reading `va` works (the frame it maps), or faults (-1). */
static long long frame(uintptr_t va)
{
    if (sigsetjmp(jb, 1))
        return -1;
    (void)*(volatile unsigned char *)va;
    off64_t off;
    size_t contig;
    if (mem_offset64((void *)va, NOFD, 1, &off, &contig) == -1)
        return -2;
    return off;
}

static int attach(void)
{
    return TraceEvent(_NTO_TRACE_LOGGER_ATTACH, NULL, NULL) == -1 ? -errno : 0;
}

/* WAITBUFFER on CPU `a` to the end; 0 once it says so. */
static void *drain(void *a)
{
    int idx, r, n = 0;
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << (intptr_t)a));
    while ((r = TraceEvent(_NTO_TRACE_WAITBUFFER, &idx)) == 0 && n++ < 1000)
        ;
    return (void *)(intptr_t)(r != _TRACE_WAITBUFFER_DONE);
}

/* A child that runs `f` and exits with its result. */
static int in_child(int (*f)(uintptr_t), uintptr_t a)
{
    pid_t p = fork();
    if (p == 0)
        _exit(f(a));
    int st;
    waitpid(p, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : -1;
}

static int faults(uintptr_t a)
{
    return frame(a) == -1;
}

static int not_logger(uintptr_t a)
{
    (void)a;
    return sys_trace(_SYS_TRACE_DEALLOCBUF, 0) == -EPERM && sys_trace(_SYS_TRACE_DETACH, 0) == 0;
}

static int logger_ends(uintptr_t a)
{
    (void)a;
    return attach() == 0 && sys_trace(_SYS_TRACE_ALLOCBUF, 1) == 0;
}

int main(void)
{
    struct sigaction sa = {.sa_handler = on_sig};
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);
    int ncpu = sysconf(_SC_NPROCESSORS_ONLN);

    /* The suite's tracelogger runs may have left a logger: ours now. */
    check("ALLOCBUF before attaching: EPERM", attach() == 0 && sys_trace(_SYS_TRACE_DETACH, 0) == 0 &&
                                                  sys_trace(_SYS_TRACE_ALLOCBUF, 1) == -EPERM);
    check("attach; ALLOCBUF of 0: EINVAL", attach() == 0 && sys_trace(_SYS_TRACE_ALLOCBUF, 0) == -EINVAL);
    check("ALLOCBUF past 32 bits of buffers: EOVERFLOW", sys_trace(_SYS_TRACE_ALLOCBUF, 0x40000000) == -EOVERFLOW);

    int r = sys_trace(_SYS_TRACE_ALLOCBUF, 2);
    uintptr_t a = got;
    long long fa = frame(a);
    check("ALLOCBUF of 2: mapped", r == 0 && fa >= 0);
    check("ALLOCBUF again: EEXIST, the buffers stay", sys_trace(_SYS_TRACE_ALLOCBUF, 4) == -EEXIST && frame(a) == fa);

    char *ni = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_NOINHERIT, NOFD, 0);
    char *ns = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON | MAP_NOINHERIT, NOFD, 0);
    ni[0] = ns[0] = 1;
    check("a fork's child has no view of them", in_child(faults, a) == 1);
    check("nor MAP_NOINHERIT mappings, private or shared",
          in_child(faults, (uintptr_t)ni) == 1 && in_child(faults, (uintptr_t)ns) == 1);
    check("another process: DEALLOCBUF EPERM, DETACH does nothing", in_child(not_logger, 0) == 1 && frame(a) == fa);

    check("DEALLOCBUF while tracing: EBUSY", TraceEvent(_NTO_TRACE_STARTNOSTATE) != -1 &&
                                                 sys_trace(_SYS_TRACE_DEALLOCBUF, 0) == -EBUSY);
    check("stopped, the buffers not taken: EBUSY", TraceEvent(_NTO_TRACE_STOP) != -1 &&
                                                       sys_trace(_SYS_TRACE_DEALLOCBUF, 0) == -EBUSY &&
                                                       frame(a) == fa);
    pthread_t t[8];
    int drained = 1;
    for (int i = 0; i < ncpu && i < 8; i++)
        pthread_create(&t[i], NULL, drain, (void *)(intptr_t)i);
    for (int i = 0; i < ncpu && i < 8; i++) {
        void *e;
        pthread_join(t[i], &e);
        drained &= e == NULL;
    }
    check("WAITBUFFER on each CPU to the end", drained);
    check("then DEALLOCBUF: the view is gone", sys_trace(_SYS_TRACE_DEALLOCBUF, 0) == 0 && frame(a) == -1);
    check("DEALLOCBUF with none: 0", sys_trace(_SYS_TRACE_DEALLOCBUF, 0) == 0);

    r = sys_trace(_SYS_TRACE_ALLOCBUF, 1);
    a = got;
    check("the logger's DETACH frees them", r == 0 && sys_trace(_SYS_TRACE_DETACH, 0) == 0 && frame(a) == -1);
    check("and ends its being the logger: ALLOCBUF EPERM", sys_trace(_SYS_TRACE_ALLOCBUF, 1) == -EPERM);

    check("a logger that ends with buffers", in_child(logger_ends, 0) == 1);
    r = attach();
    check("frees them: the next one allocates", r == 0 && sys_trace(_SYS_TRACE_ALLOCBUF, 1) == 0 &&
                                                   sys_trace(_SYS_TRACE_DETACH, 0) == 0);

    printf("traceviewtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

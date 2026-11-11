/* Stack guard pages, as procnto lays them out: the main thread's stack and
 * a pthread's (libc maps it with MAP_STACK, the guard's size as offset)
 * each start with an inaccessible page that /proc counts in the stack:
 * touching it is SIGSEGV, SEGV_MAPERR; the page above is the stack.
 * Prints one "ok"/"FAIL" line per check and "stacktest: passed". */
#include <devctl.h>
#include <fcntl.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/procfs.h>
#include <unistd.h>

static int fails;
static sigjmp_buf jb;
static volatile int code;

static void check(const char *what, int ok)
{
    printf("stacktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void on_fault(int sig, siginfo_t *si, void *ctx)
{
    (void)sig;
    (void)ctx;
    code = si->si_code;
    siglongjmp(jb, 1);
}

/* 0 if `a` reads, else the SIGSEGV code. */
static int touch(unsigned long a)
{
    code = 0;
    if (sigsetjmp(jb, 1) == 0) {
        volatile char c = *(volatile char *)a;
        (void)c;
        return 0;
    }
    return code;
}

static procfs_status status(int tid)
{
    int fd = open("/proc/self/as", O_RDONLY);
    procfs_status st;
    memset(&st, 0, sizeof st);
    st.tid = tid;
    devctl(fd, DCMD_PROC_TIDSTATUS, &st, sizeof st, NULL);
    close(fd);
    return st;
}

static void *thread(void *arg)
{
    (void)arg;
    procfs_status st = status(gettid());
    check("a thread's stack: 256 KB and a guard page", st.stksize == 0x41000);
    check("its guard: SEGV_MAPERR", touch(st.stkbase) == SEGV_MAPERR);
    check("its stack above", touch(st.stkbase + 0x1000) == 0);
    return NULL;
}

int main(void)
{
    static char alt[65536];
    stack_t ss = {alt, sizeof alt, 0};
    sigaltstack(&ss, NULL);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_fault;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigaction(SIGSEGV, &sa, NULL);

    procfs_status st = status(1);
    check("the main stack: 512 KB and a guard page", st.stksize == 0x81000);
    check("its guard: SEGV_MAPERR", touch(st.stkbase) == SEGV_MAPERR && touch(st.stkbase + 0xfff) == SEGV_MAPERR);
    check("its stack above", touch(st.stkbase + 0x1000) == 0);
    pthread_t t;
    pthread_create(&t, NULL, thread, NULL);
    pthread_join(t, NULL);
    printf("stacktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

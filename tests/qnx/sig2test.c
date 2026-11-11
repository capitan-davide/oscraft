/* Signals and blocked threads, faults, alternate stacks (as procnto):
 * - a signal with a handler ends a MsgReceive() wait with EINTR;
 * - and a MsgSend() still SEND-blocked (the server hasn't received it);
 * - REPLY-blocked: at once (EINTR) if the channel lacks _NTO_CHF_UNBLOCK,
 *   else the server gets a _PULSE_CODE_UNBLOCK pulse and the client
 *   waits for its reply (here MsgError(EINTR));
 * - a fault is a signal its handler can catch: SIGSEGV for a read of an
 *   unmapped address (si_addr that address), SIGILL for an undefined
 *   instruction;
 * - SA_ONSTACK runs a handler on the sigaltstack() stack; a stack whose
 *   end overflows the address space is EFAULT (review item SCHED-5);
 * - sigpending() (SignalProcmask(SIG_PENDING)): the blocked signals
 *   pending for the thread and the process.
 * Prints one "ok"/"FAIL" line per check and "sig2test: passed". */
#include <errno.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("sig2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static volatile int handled;

static void on_usr1(int sig)
{
    (void)sig;
    handled++;
}

/* --- message passing ------------------------------------------------- */

static int chid, coid;
static volatile int result, result_errno, started;

static void *receiver(void *arg)
{
    (void)arg;
    char buf[16];
    started = 1;
    result = MsgReceive(chid, buf, sizeof buf, NULL);
    result_errno = errno;
    return NULL;
}

static void *sender(void *arg)
{
    (void)arg;
    char buf[16] = "hello";
    started = 1;
    result = MsgSend(coid, buf, 6, buf, sizeof buf);
    result_errno = errno;
    return NULL;
}

/* Run `fn` in a thread, signal it once it is blocked, join it. */
static void signal_blocked(void *(*fn)(void *), int usleep_before)
{
    pthread_t t;
    started = 0;
    result = 12345;
    pthread_create(&t, NULL, fn, NULL);
    while (!started) {
        usleep(1000);
    }
    usleep(usleep_before);
    pthread_kill(t, SIGUSR1);
    pthread_join(t, NULL);
}

static void messages(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_usr1;
    sigaction(SIGUSR1, &sa, NULL);

    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    handled = 0;
    signal_blocked(receiver, 20000);
    check("a signal ends MsgReceive() with EINTR", result == -1 && result_errno == EINTR && handled == 1);

    handled = 0;
    signal_blocked(sender, 20000);
    check("a signal ends a SEND-blocked MsgSend() with EINTR", result == -1 && result_errno == EINTR && handled == 1);

    /* REPLY-blocked, no _NTO_CHF_UNBLOCK: unblocked at once. */
    pthread_t t;
    char buf[16];
    started = 0;
    result = 12345;
    pthread_create(&t, NULL, sender, NULL);
    int rcvid = MsgReceive(chid, buf, sizeof buf, NULL);
    usleep(20000);
    handled = 0;
    pthread_kill(t, SIGUSR1);
    pthread_join(t, NULL);
    check("a signal ends a REPLY-blocked MsgSend() at once (no _NTO_CHF_UNBLOCK)",
          result == -1 && result_errno == EINTR && handled == 1);
    int r = MsgReply(rcvid, 0, NULL, 0);
    check("the server's late reply finds no client (ESRCH)", r == -1 && errno == ESRCH);
    ConnectDetach(coid);
    ChannelDestroy(chid);

    /* REPLY-blocked, _NTO_CHF_UNBLOCK: the server is told, decides. */
    chid = ChannelCreate(_NTO_CHF_UNBLOCK);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    started = 0;
    result = 12345;
    pthread_create(&t, NULL, sender, NULL);
    rcvid = MsgReceive(chid, buf, sizeof buf, NULL);
    usleep(20000);
    handled = 0;
    pthread_kill(t, SIGUSR1);
    struct _pulse p;
    int pr = MsgReceive(chid, &p, sizeof p, NULL);
    int pulse_ok = pr == 0 && p.code == _PULSE_CODE_UNBLOCK && p.value.sival_int == rcvid;
    usleep(20000);
    int still = result == 12345;
    MsgError(rcvid, EINTR);
    pthread_join(t, NULL);
    check("_NTO_CHF_UNBLOCK: the server gets a _PULSE_CODE_UNBLOCK pulse for the rcvid", pulse_ok);
    check("_NTO_CHF_UNBLOCK: the client waits for the server's answer", still && result == -1 && result_errno == EINTR);
    ConnectDetach(coid);
    ChannelDestroy(chid);
}

/* --- faults ---------------------------------------------------------- */

static sigjmp_buf jump;
static volatile int fault_sig, fault_code;
static void *volatile fault_addr;

static void on_fault(int sig, siginfo_t *info, void *ctx)
{
    (void)ctx;
    fault_sig = sig;
    fault_code = info->si_code;
    fault_addr = info->si_addr;
    siglongjmp(jump, 1);
}

static void faults(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_fault;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGILL, &sa, NULL);

    volatile int *bad = (volatile int *)0x8;
    fault_sig = 0;
    if (sigsetjmp(jump, 1) == 0) {
        (void)*bad;
    }
    check("a read of an unmapped address: SIGSEGV to its handler", fault_sig == SIGSEGV);
    check("si_addr is the address", fault_addr == (void *)0x8);
    printf("sig2test:   SIGSEGV si_code %d\n", fault_code);

    fault_sig = 0;
    if (sigsetjmp(jump, 1) == 0) {
        __asm__ volatile(".inst 0x00000000"); /* permanently undefined */
    }
    check("an undefined instruction: SIGILL to its handler", fault_sig == SIGILL);
    printf("sig2test:   SIGILL si_code %d\n", fault_code);
}

/* --- alternate stack ------------------------------------------------- */

static char altstack[64 * 1024];
static volatile uintptr_t handler_sp;
static volatile int handler_ss_flags = -1;

static void on_usr2(int sig)
{
    (void)sig;
    int here;
    handler_sp = (uintptr_t)&here;
    stack_t o;
    if (sigaltstack(NULL, &o) == 0) {
        handler_ss_flags = o.ss_flags;
    }
}

static void alternate_stack(void)
{
    stack_t ss = { .ss_sp = altstack, .ss_size = sizeof altstack, .ss_flags = 0 };
    check("sigaltstack()", sigaltstack(&ss, NULL) == 0);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_usr2;
    sa.sa_flags = SA_ONSTACK;
    sigaction(SIGUSR2, &sa, NULL);
    raise(SIGUSR2);
    check("SA_ONSTACK: the handler runs on the alternate stack",
          handler_sp >= (uintptr_t)altstack && handler_sp < (uintptr_t)altstack + sizeof altstack);
    check("sigaltstack() in the handler: SS_ONSTACK", handler_ss_flags == SS_ONSTACK);
    stack_t small = { .ss_sp = altstack, .ss_size = 1024, .ss_flags = 0 };
    errno = 0;
    check("a stack below MINSIGSTKSZ: ENOMEM", sigaltstack(&small, NULL) == -1 && errno == ENOMEM);
    /* a stack whose end overflows the address space (review item
     * SCHED-5: the debug kernel used to panic on the arithmetic) */
    stack_t over = { .ss_sp = (void *)(~(uintptr_t)0 - 100), .ss_size = 4096, .ss_flags = 0 };
    check("a stack ending past the address space: EFAULT", sigaltstack(&over, NULL) == -1 && errno == EFAULT);
    over.ss_sp = (void *)(~(uintptr_t)0 - 4095);
    over.ss_size = 8192;
    check("another: EFAULT", sigaltstack(&over, NULL) == -1 && errno == EFAULT);
    stack_t off = { .ss_flags = SS_DISABLE }, old;
    check("SS_DISABLE, the old stack reported", sigaltstack(&off, &old) == 0 && old.ss_sp == altstack
                                                   && old.ss_size == sizeof altstack && old.ss_flags == 0);
    check("then none (SS_DISABLE)", sigaltstack(NULL, &old) == 0 && old.ss_flags == SS_DISABLE);
}

static void pending(void)
{
    sigset_t block, set;
    sigemptyset(&block);
    sigaddset(&block, SIGUSR2);
    sigaddset(&block, SIGHUP);
    sigprocmask(SIG_BLOCK, &block, NULL);
    sigemptyset(&set);
    check("sigpending() with nothing pending", sigpending(&set) == 0 && !sigismember(&set, SIGUSR2));
    pthread_kill(pthread_self(), SIGUSR2);
    kill(getpid(), SIGHUP);
    sigemptyset(&set);
    check("sigpending(): a thread's and a process's blocked signals",
          sigpending(&set) == 0 && sigismember(&set, SIGUSR2) && sigismember(&set, SIGHUP)
              && !sigismember(&set, SIGUSR1));
    signal(SIGUSR2, SIG_IGN);
    signal(SIGHUP, SIG_IGN);
    sigprocmask(SIG_UNBLOCK, &block, NULL);
}

int main(void)
{
    messages();
    faults();
    alternate_stack();
    pending();
    printf("sig2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

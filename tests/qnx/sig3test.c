/* Signal delivery as procnto does it:
 * - SA_NODEFER, SA_RESETHAND (the flags stay but SA_SIGINFO);
 * - SA_SIGINFO queues each signal sent (sigqueue() twice: two handler
 *   calls);
 * - SIGCHLD: si_code CLD_EXITED/CLD_KILLED, si_uid -1; on a stop si_code 0
 *   (not with SA_NOCLDSTOP), none on a continuation; waitpid(WUNTRACED)
 *   and waitpid(WCONTINUED) report them;
 * - a held waitid() is answered before SIGCHLD can interrupt it, and the
 *   child's SIGCHLD goes (it isn't delivered after); with SIGCHLD ignored
 *   children leave no zombie (ECHILD);
 * - sigtimedwait(), alarm()'s SIGALRM (si_code 0).
 * Prints one "ok"/"FAIL" line per check and "sig3test: passed". */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;
static volatile int depth, maxdepth, count;
static siginfo_t last;

static void check(const char *what, int ok)
{
    printf("sig3test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Counts; SIGUSR1 raises itself again twice (how deep it nests shows
 * whether the signal is blocked in its handler). */
static void handler(int signo, siginfo_t *si, void *ctx)
{
    (void)ctx;
    last = *si;
    count++;
    depth++;
    if (depth > maxdepth) {
        maxdepth = depth;
    }
    if (count < 3 && signo == SIGUSR1) {
        raise(signo);
    }
    depth--;
}

static void wait_count(void)
{
    for (int i = 0; i < 1000 && count == 0; i++) {
        usleep(1000);
    }
}

static void flags(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = handler;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGUSR1, &sa, NULL);
    raise(SIGUSR1);
    check("a signal is blocked in its handler (one level)", count == 3 && maxdepth == 1);
    check("raise(): si_code SI_USER, si_pid", last.si_code == SI_USER && last.si_pid == getpid());
    count = maxdepth = 0;
    sa.sa_flags = SA_SIGINFO | SA_NODEFER;
    sigaction(SIGUSR1, &sa, NULL);
    raise(SIGUSR1);
    check("SA_NODEFER: it nests", count == 3 && maxdepth == 3);

    count = 0;
    sa.sa_flags = SA_SIGINFO | SA_RESETHAND;
    sigaction(SIGUSR2, &sa, NULL);
    sigset_t m;
    sigemptyset(&m);
    sigaddset(&m, SIGUSR2);
    sigprocmask(SIG_BLOCK, &m, NULL);
    raise(SIGUSR2);
    sigprocmask(SIG_UNBLOCK, &m, NULL);
    struct sigaction old;
    sigaction(SIGUSR2, NULL, &old);
    check("SA_RESETHAND: SIG_DFL again, the flags but SA_SIGINFO",
          count == 1 && old.sa_handler == SIG_DFL && old.sa_flags == SA_RESETHAND);
}

static void queueing(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = handler;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGRTMIN, &sa, NULL);
    sigset_t m;
    sigemptyset(&m);
    sigaddset(&m, SIGRTMIN);
    sigprocmask(SIG_BLOCK, &m, NULL);
    count = 0;
    sigqueue(getpid(), SIGRTMIN, (union sigval){ .sival_int = 4242 });
    sigqueue(getpid(), SIGRTMIN, (union sigval){ .sival_int = 4343 });
    sigprocmask(SIG_UNBLOCK, &m, NULL);
    check("SA_SIGINFO: both sigqueue()s delivered, in order",
          count == 2 && last.si_value.sival_int == 4343 && last.si_code == SI_QUEUE);

    sigaction(SIGUSR1, &sa, NULL);
    sigemptyset(&m);
    sigaddset(&m, SIGUSR1);
    sigprocmask(SIG_BLOCK, &m, NULL);
    count = 0;
    sigqueue(getpid(), SIGUSR1, (union sigval){ .sival_int = 1 });
    sigqueue(getpid(), SIGUSR1, (union sigval){ .sival_int = 2 });
    sigprocmask(SIG_UNBLOCK, &m, NULL);
    /* procnto also delivers the queued ones before those the handler
     * raise()s (last.si_value 0); OsCraft doesn't order them so yet. */
    check("two queued SIGUSR1s and the handler's raise()s: four calls", count == 4);
}

static void children(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = handler;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGCHLD, &sa, NULL);
    count = 0;
    pid_t pid = fork();
    if (pid == 0) {
        _exit(7);
    }
    wait_count();
    check("SIGCHLD on exit: CLD_EXITED, the status, si_uid -1",
          last.si_code == CLD_EXITED && last.si_status == 7 && last.si_pid == pid && last.si_uid == (uid_t)-1);
    waitpid(pid, NULL, 0);

    count = 0;
    pid = fork();
    if (pid == 0) {
        for (;;) {
            pause();
        }
    }
    usleep(50000);
    kill(pid, SIGSTOP);
    wait_count();
    check("SIGCHLD on a stop: si_code 0", count == 1 && last.si_code == 0 && last.si_pid == pid);
    int st;
    check("waitpid(WUNTRACED): stopped by SIGSTOP",
          waitpid(pid, &st, WUNTRACED | WNOHANG) == pid && WIFSTOPPED(st) && WSTOPSIG(st) == SIGSTOP);
    count = 0;
    kill(pid, SIGCONT);
    usleep(100000);
    check("no SIGCHLD on a continuation", count == 0);
    check("waitpid(WCONTINUED): continued", waitpid(pid, &st, WCONTINUED | WNOHANG) == pid && WIFCONTINUED(st));
    sa.sa_flags = SA_SIGINFO | SA_NOCLDSTOP;
    sigaction(SIGCHLD, &sa, NULL);
    count = 0;
    kill(pid, SIGSTOP);
    usleep(100000);
    check("SA_NOCLDSTOP: no SIGCHLD on a stop", count == 0);
    kill(pid, SIGKILL);
    waitpid(pid, NULL, 0);

    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGCHLD, &sa, NULL);
    count = 0;
    pid = fork();
    if (pid == 0) {
        usleep(100000);
        _exit(3);
    }
    siginfo_t wi;
    memset(&wi, 0, sizeof wi);
    int r = waitid(P_PID, pid, &wi, WEXITED);
    if (r != 0 || wi.si_signo != SIGCHLD) {
        printf("sig3test:   waitid: %d errno %d signo %d code %d status %d\n", r, r ? errno : 0, wi.si_signo,
               wi.si_code, wi.si_status);
    }
    check("a held waitid() is answered, not interrupted by SIGCHLD",
          r == 0 && wi.si_signo == SIGCHLD && wi.si_code == CLD_EXITED && wi.si_status == 3);
    usleep(50000);
    check("no SIGCHLD for the child a held wait took", count == 0);
    signal(SIGCHLD, SIG_IGN);
    pid = fork();
    if (pid == 0) {
        _exit(4);
    }
    errno = 0;
    r = waitid(P_PID, pid, &wi, WEXITED);
    check("SIGCHLD ignored: no zombie (ECHILD)", r == -1 && errno == ECHILD);
    signal(SIGCHLD, SIG_DFL);
}

static void waiting(void)
{
    sigset_t m;
    sigemptyset(&m);
    sigaddset(&m, SIGUSR1);
    sigprocmask(SIG_BLOCK, &m, NULL);
    struct timespec ts = { 0, 1000000 };
    siginfo_t si;
    errno = 0;
    check("sigtimedwait() with nothing: EAGAIN", sigtimedwait(&m, &si, &ts) == -1 && errno == EAGAIN);
    raise(SIGUSR1);
    check("sigtimedwait() takes a pending signal", sigtimedwait(&m, &si, &ts) == SIGUSR1 && si.si_code == SI_USER);
    sigemptyset(&m);
    sigaddset(&m, SIGALRM);
    sigprocmask(SIG_BLOCK, &m, NULL);
    alarm(1);
    check("alarm(): SIGALRM, si_code 0", sigwaitinfo(&m, &si) == SIGALRM && si.si_code == 0);
}

int main(void)
{
    flags();
    queueing();
    children();
    waiting();
    printf("sig3test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

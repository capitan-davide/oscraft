/* QNX's special signals (57-64, SIGSELECT): always blocked, not to be
 * caught or ignored, and kept pending until sigwaitinfo() takes them, as
 * poll() relies on. Signals to one thread (pthread_kill()). Prints one
 * "ok"/"FAIL" line per case. */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("sigtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void handler(int signo) { (void)signo; }

static void *waiter(void *arg)
{
    sigset_t *set = arg;
    int signo = 0;
    sigwait(set, &signo);
    return (void *)(long)signo;
}

static void *sleeper(void *arg)
{
    (void)arg;
    usleep(300 * 1000);
    return NULL;
}

int main(void)
{
    sigset_t set, old;
    sigemptyset(&set);
    sigaddset(&set, SIGSELECT);
    sigprocmask(SIG_UNBLOCK, &set, NULL);
    sigprocmask(SIG_BLOCK, NULL, &old);
    check("can't be unblocked", sigismember(&old, SIGSELECT) == 1);

    struct sigaction sa = {0};
    sa.sa_handler = handler;
    check("can't be caught", sigaction(SIGSELECT, &sa, NULL) == -1 && errno == EINVAL);
    sa.sa_handler = SIG_IGN;
    check("can't be ignored", sigaction(SIGSELECT, &sa, NULL) == -1 && errno == EINVAL);

    /* Sent before anyone waits (a server's notification before poll()
     * waits): still there for sigwaitinfo(). */
    check("sent", SignalKill(0, getpid(), 0, SIGSELECT, 0, 0) == 0);
    siginfo_t info;
    struct timespec zero = {0, 0};
    check("kept until sigwaitinfo()", sigtimedwait(&set, &info, &zero) == SIGSELECT);

    /* SIGUSR1 blocked in all threads; pthread_kill() picks one. */
    sigset_t usr1;
    sigemptyset(&usr1);
    sigaddset(&usr1, SIGUSR1);
    pthread_sigmask(SIG_BLOCK, &usr1, NULL);
    pthread_t w, z;
    void *got = NULL;
    pthread_create(&w, NULL, waiter, &usr1);
    usleep(100 * 1000);
    check("pthread_kill", pthread_kill(w, SIGUSR1) == 0);
    pthread_join(w, &got);
    check("the thread got it", (long)got == SIGUSR1);
    pthread_create(&z, NULL, sleeper, NULL);
    pthread_kill(z, SIGUSR1);
    check("another thread's signal isn't ours", sigtimedwait(&usr1, &info, &zero) == -1 && errno == EAGAIN);
    pthread_join(z, NULL);

    /* alarm(): SIGALRM after the time, reset by each call. */
    sigset_t alrm;
    sigemptyset(&alrm);
    sigaddset(&alrm, SIGALRM);
    sigprocmask(SIG_BLOCK, &alrm, NULL);
    alarm(5);
    check("alarm() returns the time left", alarm(1) >= 4);
    struct timespec two = {2, 0};
    check("SIGALRM", sigtimedwait(&alrm, &info, &two) == SIGALRM);
    alarm(1);
    check("alarm(0) cancels", alarm(0) == 1 && sigtimedwait(&alrm, &info, &two) == -1);

    printf("sigtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

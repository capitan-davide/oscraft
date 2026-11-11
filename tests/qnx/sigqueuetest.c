/* Queued signals and memory (review item SCHED-8). procnto queues a
 * blocked SA_SIGINFO signal without a count limit, until memory runs out,
 * then sigqueue() fails with ENOMEM (measured: 50 million, about 40 bytes
 * each). The kernel queued them in its heap until an allocation failed
 * and it panicked. This maps all but 48 MiB of the free memory, queues
 * until sigqueue() fails, and checks: ENOMEM; every signal queued is then
 * taken once; the system still works (a fork() and its wait); sigqueue()
 * works again. procnto passes the first checks (ENOMEM after 876793 here)
 * but then no longer answers: it keeps no memory back from the queues.
 * OsCraft keeps the kernel's reserve, so the rest checks it survives.
 * Prints "sigqueuetest: passed". */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define LEFT (48L << 20)

static int fails;

static void check(const char *what, int ok)
{
    printf("sigqueuetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void h(int s, siginfo_t *i, void *c)
{
    (void)s;
    (void)i;
    (void)c;
}

static long freemem(void)
{
    struct stat st;
    return stat("/proc", &st) == 0 ? (long)st.st_size : 0;
}

int main(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = h;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGRTMIN, &sa, NULL);
    sigset_t b;
    sigemptyset(&b);
    sigaddset(&b, SIGRTMIN);
    sigprocmask(SIG_BLOCK, &b, NULL);

    long take = freemem() - LEFT;
    void *hog = take > 0 ? mmap(NULL, take, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0) : MAP_FAILED;
    check("all but 48 MiB of free memory mapped", hog != MAP_FAILED);

    union sigval v = { .sival_int = 7 };
    long n = 0;
    int err = 0;
    while (n < 20000000) {
        if (sigqueue(getpid(), SIGRTMIN, v) == -1) {
            err = errno;
            break;
        }
        n++;
    }
    printf("sigqueuetest:   %ld queued, then errno %d\n", n, err);
    check("sigqueue() fails with ENOMEM once memory is short", err == ENOMEM);
    check("after many queued (over 100000)", n > 100000);

    struct timespec z = { 0, 0 };
    long got = 0;
    siginfo_t si;
    int vals = 1;
    while (sigtimedwait(&b, &si, &z) == SIGRTMIN) {
        got++;
        vals &= si.si_value.sival_int == 7;
    }
    if (hog != MAP_FAILED)
        munmap(hog, take);
    check("every signal queued taken once, with its value", got == n && vals);

    pid_t c = fork();
    if (c == 0)
        _exit(3);
    int st = 0;
    check("then a fork() and its wait work", c > 0 && waitpid(c, &st, 0) == c && WEXITSTATUS(st) == 3);
    check("and sigqueue() again", sigqueue(getpid(), SIGRTMIN, v) == 0 && sigtimedwait(&b, &si, &z) == SIGRTMIN);
    printf("sigqueuetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

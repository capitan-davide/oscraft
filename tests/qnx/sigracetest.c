/* sigtimedwait() whose timeout expires as a signal arrives (review item
 * SCHED-17). Two answers are right: the signal (taken), or EAGAIN with
 * the signal still pending, taken by the next wait. Whichever, each
 * signal is received exactly once, with its value. The kernel let a
 * timeout that fired after a signal had already woken the thread store
 * ETIMEDOUT over that wake's reason; it now stores it only if its own
 * wake succeeds. Many rounds, the signal sent from another thread close
 * to the deadline. procnto now and then returns 0 rather than the signal
 * number in this race, having taken the signal (a procnto glitch, not
 * copied: issue #266). Prints one "ok"/"FAIL" line per check and
 * "sigracetest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <time.h>
#include <unistd.h>

#define ROUNDS 1500

static int fails;
static volatile int go, round_no;

static void check(const char *what, int ok)
{
    printf("sigracetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* queues SIGUSR1 with the round's value to the process (both threads
 * block it: the waiter takes it), about when the waiter's 1 ms timeout
 * expires */
static void *sender(void *a)
{
    (void)a;
    for (;;) {
        while (!go)
            ;
        go = 0;
        struct timespec ts = { 0, 900000 + (round_no % 7) * 50000 };
        nanosleep(&ts, NULL);
        union sigval v = { .sival_int = round_no };
        sigqueue(getpid(), SIGUSR1, v);
    }
    return NULL;
}

int main(void)
{
    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    pthread_sigmask(SIG_BLOCK, &set, NULL);
    struct sched_param sp = { .sched_priority = 12 };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    pthread_t s;
    pthread_create(&s, NULL, sender, NULL);
    int taken = 0, timed_out = 0, missing = 0, wrong = 0, other = 0;
    for (int r = 1; r <= ROUNDS; r++) {
        round_no = r;
        go = 1;
        siginfo_t si;
        si.si_signo = -1;
        si.si_value.sival_int = -1;
        struct timespec to = { 0, 1000000 };
        int n = sigtimedwait(&set, &si, &to);
        /* procnto (measured) now and then returns 0 in this race, the
         * signal taken and its siginfo filled in: counted as taken */
        if (n == SIGUSR1 || (n == 0 && si.si_signo == SIGUSR1)) {
            taken++;
        } else if (n == -1 && errno == EAGAIN) {
            timed_out++;
            /* the signal comes, or has come: the next wait takes it */
            struct timespec longer = { 1, 0 };
            n = sigtimedwait(&set, &si, &longer);
            if (n != SIGUSR1) {
                missing++;
                continue;
            }
        } else {
            if (other++ < 5)
                printf("sigracetest: round %d: %d, errno %d\n", r, n, errno);
            continue;
        }
        wrong += si.si_value.sival_int != r;
    }
    struct timespec none = { 0, 0 };
    siginfo_t si;
    int extra = sigtimedwait(&set, &si, &none) == SIGUSR1;
    printf("sigracetest: %d rounds: taken by the timed wait %s, after its timeout %s\n", ROUNDS,
           taken ? "sometimes" : "never", timed_out ? "sometimes" : "never");
    check("every signal received", missing == 0);
    check("each with its own round's value", wrong == 0);
    check("no other result", other == 0);
    check("none left over", !extra);
    printf("sigracetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

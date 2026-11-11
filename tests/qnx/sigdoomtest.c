/* A caught signal for a thread whose stack has no room for the handler's
 * frame (its stack pointer is 64), as procnto (measured; review item
 * SCHED-30): the process ends, killed by SIGDOOM ("target thread unable
 * to process signal"), whether the signal was sent to the process or to
 * the thread. Each case in a child. Prints one "ok"/"FAIL" line per check
 * and "sigdoomtest: passed". */
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("sigdoomtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void on_usr1(int s)
{
    (void)s;
}

static void *tiny(void *a)
{
    (void)a;
    sigset_t none;
    sigemptyset(&none);
    pthread_sigmask(SIG_SETMASK, &none, NULL);
    __asm__ volatile("mov sp, %0\n2: b 2b" ::"r"((uintptr_t)64));
    return NULL;
}

/* How a child ends whose thread with a tiny stack is sent SIGUSR1, to the
 * process (`to_thread` 0) or to the thread. */
static int ends(int to_thread)
{
    pid_t p = fork();
    if (p == 0) {
        signal(SIGUSR1, on_usr1);
        sigset_t all;
        sigfillset(&all);
        pthread_sigmask(SIG_SETMASK, &all, NULL); /* only the tiny thread takes it */
        pthread_t t;
        pthread_create(&t, NULL, tiny, NULL);
        usleep(20000);
        if (to_thread)
            pthread_kill(t, SIGUSR1);
        else
            kill(getpid(), SIGUSR1);
        sleep(2);
        _exit(0);
    }
    int st;
    waitpid(p, &st, 0);
    return WIFSIGNALED(st) ? WTERMSIG(st) : -1;
}

int main(void)
{
    check("kill(): SIGDOOM", ends(0) == SIGDOOM);
    check("pthread_kill(): SIGDOOM", ends(1) == SIGDOOM);
    printf("sigdoomtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

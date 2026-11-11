/* Thread creation on a stack the process can't write (review item
 * SCHED-12), as procnto (measured): pthread_create() returns 0, the new
 * thread faults as it uses its stack and the process dies of SIGSEGV, for
 * a read-only stack and for one in unmapped memory. Thread IDs are reused:
 * a process whose first extra thread has ended gets ID 2 again. The kernel
 * used to fail the call with EFAULT and keep the thread ID it had taken,
 * so the next thread got a higher one. Each case runs in a child. Prints
 * one "ok"/"FAIL" line per check and "tidtest: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("tidtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *body(void *a)
{
    (void)a;
    return NULL;
}

/* in a child: create a thread on `stack`; exit 0 if pthread_create()
 * failed (it shouldn't), else wait to be killed */
static int child(void *stack)
{
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setstack(&a, stack, 1 << 16);
    pthread_t t;
    int r = pthread_create(&t, &a, body, NULL);
    if (r != 0) {
        _exit(100 + r);
    }
    sleep(2);
    _exit(0);
}

int main(void)
{
    void *ro = mmap(NULL, 1 << 16, PROT_READ, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    void *cases[] = { ro, (void *)0x1000 };
    const char *names[] = { "a read-only stack", "a stack in unmapped memory" };
    for (int i = 0; i < 2; i++) {
        pid_t c = fork();
        if (c == 0) {
            child(cases[i]);
        }
        int st = 0;
        waitpid(c, &st, 0);
        char what[120];
        snprintf(what, sizeof what, "%s: the thread is created and the process dies of SIGSEGV (%s %d)", names[i],
                 WIFSIGNALED(st) ? "signal" : "exit", WIFSIGNALED(st) ? WTERMSIG(st) : WEXITSTATUS(st));
        check(what, WIFSIGNALED(st) && WTERMSIG(st) == SIGSEGV);
    }
    pthread_t t;
    pthread_create(&t, NULL, body, NULL);
    pthread_join(t, NULL);
    pthread_t t2;
    pthread_create(&t2, NULL, body, NULL);
    pthread_join(t2, NULL);
    char what[80];
    snprintf(what, sizeof what, "thread IDs reused: %d, then %d", (int)t, (int)t2);
    check(what, t == 2 && t2 == 2);
    printf("tidtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

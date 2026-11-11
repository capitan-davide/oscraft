/* exec() while another thread of the process is REPLY-blocked on procnto
 * in a request procnto answers later, as on procnto (measured):
 * - a held waitpid() (review item PROCMGR-2): the exec goes through, and
 *   the child is the exec'd program's to wait for;
 * - a fork() or a spawn whose child hasn't started yet (review item
 *   PROCMGR-26: the child has hundreds of descriptors of another server
 *   to inherit, which makes its start slow): the exec goes through, the
 *   child never runs, and nothing is left to wait for.
 * Before the fixes the manager's thread, ending the other threads, waited
 * for a pulse only it could serve, and every later request to procnto
 * hung. Each case is a process of its own that execs itself; the exec'd
 * program reports. Prints "ok"/"FAIL" lines and "execwaittest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <process.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#define SELF "/proc/boot/execwaittest"
static int fails;

static void check(const char *what, int ok)
{
    printf("execwaittest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *waiter(void *arg)
{
    (void)arg;
    int st;
    waitpid(-1, &st, 0);
    return NULL;
}

static void *forker(void *arg)
{
    (void)arg;
    if (fork() == 0) {
        /* never reached on either kernel: the child is abandoned */
        printf("execwaittest: FAIL the abandoned fork child ran\n");
        _exit(0);
    }
    return NULL;
}

static void *spawner(void *arg)
{
    (void)arg;
    spawnl(P_NOWAIT, SELF, "execwaittest", "spawned", NULL);
    return NULL;
}

/* a case: hold a sibling in `held`, then exec into `next` */
static int held_case(void *(*held)(void *), const char *next, int many_fds)
{
    if (many_fds) {
        for (int i = 0; i < 400; i++) {
            open("/dev/ser1", O_RDONLY);
        }
    }
    pthread_t t;
    pthread_create(&t, NULL, held, NULL);
    usleep(many_fds ? 1000 : 100000);
    execl(SELF, "execwaittest", next, NULL);
    printf("execwaittest: FAIL execv: errno %d\n", errno);
    return 1;
}

int main(int argc, char **argv)
{
    const char *a = argc > 1 ? argv[1] : "";
    if (strcmp(a, "wait-case") == 0) {
        if (fork() == 0) {
            sleep(2);
            _exit(0);
        }
        return held_case(waiter, "after-wait", 0);
    }
    if (strcmp(a, "after-wait") == 0) {
        /* the child of the first incarnation is ours to wait for */
        return waitpid(-1, NULL, 0) > 0 ? 0 : 1;
    }
    if (strcmp(a, "fork-case") == 0) {
        return held_case(forker, "after-held", 1);
    }
    if (strcmp(a, "spawn-case") == 0) {
        return held_case(spawner, "after-held", 1);
    }
    if (strcmp(a, "spawned") == 0) {
        printf("execwaittest: FAIL the abandoned spawned child ran\n");
        return 0;
    }
    if (strcmp(a, "after-held") == 0) {
        /* procnto serves; the abandoned child is nothing to wait for */
        int st = spawnl(P_WAIT, SELF, "execwaittest", "noop", NULL);
        pid_t r = waitpid(-1, NULL, WNOHANG);
        return st == 0 && r == -1 && errno == ECHILD ? 0 : 2;
    }
    if (strcmp(a, "noop") == 0) {
        return 0;
    }
    static const struct { const char *arg, *what; } cases[] = {
        { "wait-case", "a sibling in waitpid(): the exec'd program runs and waits for the child" },
        { "fork-case", "a sibling in fork(): the exec'd program runs, procnto serves, no child to wait for" },
        { "spawn-case", "a sibling in spawn(): the exec'd program runs, procnto serves, no child to wait for" },
    };
    for (unsigned i = 0; i < sizeof cases / sizeof *cases; i++) {
        int st = 0;
        pid_t p = spawnl(P_NOWAIT, SELF, "execwaittest", cases[i].arg, NULL);
        waitpid(p, &st, 0);
        check(cases[i].what, p > 0 && WIFEXITED(st) && WEXITSTATUS(st) == 0);
    }
    usleep(300000); /* an abandoned child that ran would have printed by now */
    printf("execwaittest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

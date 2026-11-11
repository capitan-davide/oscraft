/* A parent's wait answered while its child's last thread is still ending
 * (review item IPC-12). The child holds 64 descriptors, 16 channels with
 * connections to them and 3 more threads, so its teardown after it becomes
 * a zombie is long; the parent is already blocked in waitpid(), and the
 * wait removes the child (freeing its address space) as soon as procnto
 * answers it. 300 rounds; each status must be the child's. The kernel
 * must keep the last thread on its CPU until its final switch: freed
 * tables under a thread switched back in would show as a crash or a hang.
 * Prints "exitwaittest: passed". */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

#define ROUNDS 300

static void *spin(void *a)
{
    for (;;) {
        sched_yield();
    }
    return a;
}

static void *ender(void *a)
{
    exit((int)(long)a & 0x7f);
}

static void child(int round)
{
    for (int i = 0; i < 64; i++) {
        dup(0);
    }
    for (int i = 0; i < 16; i++) {
        int chid = ChannelCreate(_NTO_CHF_COID_DISCONNECT);
        ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    }
    pthread_t t;
    for (int i = 0; i < 3; i++) {
        pthread_create(&t, NULL, spin, NULL);
    }
    /* Every other round, ended by another thread than the first. */
    if (round % 2) {
        pthread_create(&t, NULL, ender, (void *)(long)round);
        spin(NULL);
    }
    _exit(round & 0x7f);
}

int main(void)
{
    int bad = 0;
    for (int r = 0; r < ROUNDS; r++) {
        pid_t pid = fork();
        if (pid == 0) {
            child(r);
        }
        if (pid < 0) {
            printf("exitwaittest: FAIL fork() in round %d\n", r);
            return 1;
        }
        int status;
        pid_t got = waitpid(pid, &status, 0);
        if (got != pid || !WIFEXITED(status) || WEXITSTATUS(status) != (r & 0x7f)) {
            printf("exitwaittest: FAIL round %d: waitpid %d, status %#x\n", r, got, status);
            bad++;
        }
    }
    printf("exitwaittest: %s %d children waited for, each status its own\n", bad ? "FAIL" : "ok", ROUNDS);
    printf("exitwaittest: %s\n", bad ? "FAILED" : "passed");
    return bad != 0;
}

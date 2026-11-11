/* A program to debug: a worker thread, a function to break in, and, with
 * "crash", a null pointer write; with "spin", a loop to attach to. */
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

volatile int counter;

int add(int a, int b)
{
    return a + b;
}

static void *worker(void *arg)
{
    // main must be in its pthread_join() (state JOIN, which the gdb tests
    // expect of thread 1) before the breakpoint in add() hits; on four
    // CPUs the worker can get there first (issue #220).
    usleep(20000);
    for (int i = 0; i < 3; i++) {
        counter = add(counter, i);
    }
    return arg;
}

int main(int argc, char **argv)
{
    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    pthread_join(t, NULL);
    printf("dbgtest: counter=%d\n", counter);
    if (argc > 1 && strcmp(argv[1], "crash") == 0) {
        *(volatile int *)0 = 42;
    }
    if (argc > 1 && strcmp(argv[1], "spin") == 0) {
        for (;;) {
            counter = add(counter, 1);
        }
    }
    return 0;
}

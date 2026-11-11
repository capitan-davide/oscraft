/* Mappings made into one process by two threads at once (review item
 * CORE-3): the main thread maps, fills, checks and unmaps anonymous
 * memory in a loop while a SIGEV_THREAD timer, whose threads are created
 * (and their stacks mapped) by the kernel's sigev worker, fires every
 * 2 ms; each of those threads also maps and checks a page. Before the
 * fix two placements could get the same range: a handler thread then
 * crashed on a pointer made of the main thread's fill byte, and a mmap()
 * failed with ENOMEM on the already-mapped range. procnto passes.
 * Prints "maprace: passed". */
#include <errno.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#define ROUNDS 1000
#define SIZES 4

static volatile int fails, fired;
static volatile int stop;

static int check_fill(void *p, size_t len, unsigned char v)
{
    unsigned char *b = p;
    memset(b, v, len);
    for (size_t i = 0; i < len; i += 64)
        if (b[i] != v)
            return 0;
    /* give a colliding mapper time to write over it */
    for (volatile int spin = 0; spin < 2000; spin++)
        ;
    for (size_t i = 0; i < len; i += 64)
        if (b[i] != v)
            return 0;
    return 1;
}

static void on_timer(union sigval sv)
{
    (void)sv;
    if (stop)
        return;
    fired++;
    void *p = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    if (p == MAP_FAILED) {
        printf("maprace: handler mmap failed: %s\n", strerror(errno));
        fails++;
        return;
    }
    if (!check_fill(p, 4096, 0x5a))
        fails++;
    munmap(p, 4096);
}

int main(void)
{
    static const size_t sizes[SIZES] = { 4096, 2 * 4096, 7 * 4096, 32 * 4096 };
    timer_t t;
    struct sigevent ev;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 16 * 1024);
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_THREAD;
    ev.sigev_notify_function = on_timer;
    ev.sigev_notify_attributes = &attr;
    if (timer_create(CLOCK_MONOTONIC, &ev, &t) != 0) {
        perror("maprace: timer_create");
        return 1;
    }
    struct itimerspec its = { { 0, 2000000 }, { 0, 2000000 } };
    timer_settime(t, 0, &its, NULL);
    for (int r = 0; r < ROUNDS; r++) {
        size_t len = sizes[r % SIZES];
        void *p = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
        if (p == MAP_FAILED) {
            printf("maprace: round %d mmap(%zu) failed: %s (fired %d)\n", r, len, strerror(errno), fired);
            fails++;
            continue;
        }
        if (!check_fill(p, len, (unsigned char)(r + 1)))
            fails++;
        munmap(p, len);
        if (r % 500 == 0) {
            printf("maprace: round %d, timer fired %d times\n", r, fired);
            fflush(stdout);
        }
    }
    stop = 1;
    timer_delete(t);
    usleep(20000);
    printf("maprace: timer fired %d times\n", fired);
    printf("maprace: %s\n", fails ? "FAIL" : "passed");
    return fails != 0;
}

/* Process 1's long operations leave its CPU's interrupts open (review
 * item PATHMEM-4), as procnto's: a prio-63 thread pinned to each CPU,
 * sleeping to absolute 1 ms deadlines, stays on time while a prio-10
 * client has procnto allocate 256 MiB (shm_ctl(SHMCTL_ANON), which
 * zero-fills it), free it (close()), and write 2 MiB at a time to a
 * /dev/shmem file. Measured worst lateness: procnto about 1 ms; the
 * kernel was late by the whole operation (0.5 s and more). Two threads
 * reading one descriptor still get each MiB once. Prints one "ok"/"FAIL"
 * line per check and "longoptest: passed". */
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/syspage.h>
#include <time.h>
#include <unistd.h>

#define MB (1024 * 1024)
/* Wide, for QEMU and the debug kernel: the bug was 5 to 20 times this. */
#define LATE_MAX 100000000ULL

static int fails;
static volatile int stop;
static volatile uint64_t worst[8];
static int ncpu;
static int fd;
static int seen[8], bad;

static void check(const char *what, int ok)
{
    printf("longoptest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static uint64_t now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1000000000ULL + ts.tv_nsec;
}

static void *meter(void *a)
{
    int cpu = (int)(intptr_t)a;
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
    uint64_t t = now();
    while (!stop) {
        t += 1000000;
        struct timespec d = { t / 1000000000ULL, t % 1000000000ULL };
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &d, NULL);
        uint64_t late = now() - t;
        if (late > worst[cpu])
            worst[cpu] = late;
        if (now() > t + 1000000)
            t = now();
    }
    return NULL;
}

static void reset(void)
{
    usleep(5000);
    memset((void *)worst, 0, sizeof worst);
}

static uint64_t latest(void)
{
    uint64_t w = 0;
    for (int i = 0; i < ncpu; i++)
        if (worst[i] > w)
            w = worst[i];
    return w;
}

static void *reader(void *a)
{
    (void)a;
    char *b = malloc(MB);
    ssize_t r;
    while ((r = read(fd, b, MB)) > 0) {
        if (r != MB || b[0] != b[MB - 1] || b[0] < 0 || b[0] >= 8)
            bad++;
        else
            __sync_fetch_and_add(&seen[(int)b[0]], 1);
    }
    free(b);
    return NULL;
}

int main(void)
{
    ncpu = _syspage_ptr->num_cpu > 8 ? 8 : _syspage_ptr->num_cpu;
    struct sched_param sp = { .sched_priority = 10 };
    pthread_setschedparam(pthread_self(), SCHED_FIFO, &sp);
    pthread_attr_t at;
    pthread_attr_init(&at);
    pthread_attr_setinheritsched(&at, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&at, SCHED_FIFO);
    sp.sched_priority = 63;
    pthread_attr_setschedparam(&at, &sp);
    pthread_t th[8];
    for (int i = 0; i < ncpu; i++)
        pthread_create(&th[i], &at, meter, (void *)(intptr_t)i);
    usleep(100000);

    reset();
    int shm = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    check("shm_ctl(SHMCTL_ANON, 256 MiB)", shm_ctl(shm, SHMCTL_ANON, 0, 256 * MB) == 0);
    check("a 1 ms timer on every CPU stays within 100 ms meanwhile", latest() < LATE_MAX);
    reset();
    close(shm);
    usleep(20000);
    check("and while close() frees it", latest() < LATE_MAX);

    char *buf = malloc(2 * MB);
    fd = open("/dev/shmem/longoptest", O_RDWR | O_CREAT | O_TRUNC, 0600);
    unlink("/dev/shmem/longoptest");
    reset();
    ssize_t w = 0;
    for (int i = 0; i < 4; i++) {
        for (int j = 0; j < 2; j++)
            memset(buf + j * MB, i * 2 + j, MB);
        w += write(fd, buf, 2 * MB);
    }
    check("and while 2 MiB writes go to a /dev/shmem file", latest() < LATE_MAX);
    check("all of them written", w == 8 * MB);
    stop = 1;
    for (int i = 0; i < ncpu; i++)
        pthread_join(th[i], NULL);

    if (w == 8 * MB) {
        lseek(fd, 0, SEEK_SET);
        pthread_t r1, r2;
        pthread_create(&r1, NULL, reader, NULL);
        pthread_create(&r2, NULL, reader, NULL);
        pthread_join(r1, NULL);
        pthread_join(r2, NULL);
        int once = !bad;
        for (int i = 0; i < 8; i++)
            once &= seen[i] == 1;
        check("two threads reading one descriptor get each MiB once", once);
    }
    close(fd);

    printf("longoptest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

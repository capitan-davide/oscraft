/* Kernel page tables mutated from several CPUs at once (review item
 * CORE-1). Four spawners, one per CPU, each create a growing batch of
 * threads that block until the batch is complete, then release and join
 * them. Every new thread costs the kernel a stack (fresh kernel pages)
 * and, through a private mapping of a shared-memory object, a kernel
 * heap block, so the CPUs map kernel pages concurrently and keep crossing
 * translation-table boundaries. The race it guards against (two CPUs
 * installing a table for the same entry, or zeroing through the one
 * scratch page at once) was never hit in QEMU, even with 4096 live
 * threads: it is a stress, not a reproduction. Prints one line per round
 * and "mmustress: passed". */
#include <errno.h>
#include <pthread.h>
#include <semaphore.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <unistd.h>

#define CPUS 4
#define ROUNDS 8
#define BASE 24          /* threads per spawner in round 0, doubling... */
#define MAXT 128         /* ...up to this many per spawner */

static sem_t gate[CPUS];
static int fails;
static int shmfd;
#define SHMLEN 4096

/* stdio is not used from the threads: write() only */
static void say(const char *fmt, int a, int b, int c, int d)
{
    char buf[128];
    int n = snprintf(buf, sizeof buf, fmt, a, b, c, d);
    write(1, buf, n);
}

static void pin(int cpu)
{
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
}

static void *worker(void *arg)
{
    sem_t *g = arg;
    /* a private copy of a shared-memory object per thread: user pages,
     * and on OsCraft a kernel-heap block the size of the object */
    void *p = mmap(NULL, SHMLEN, PROT_READ | PROT_WRITE, MAP_PRIVATE, shmfd, 0);
    if (p == MAP_FAILED) {
        fails++;
    } else {
        memset(p, 1, 4096);
    }
    sem_wait(g);
    if (p != MAP_FAILED)
        munmap(p, SHMLEN);
    return NULL;
}

static void *spawner(void *arg)
{
    int cpu = (int)(intptr_t)arg;
    pin(cpu);
    pthread_t t[MAXT];
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setstacksize(&attr, 16 * 1024);
    int n = BASE;
    for (int r = 0; r < ROUNDS; r++) {
        int made = 0;
        for (int i = 0; i < n; i++) {
            int e = pthread_create(&t[i], &attr, worker, &gate[cpu]);
            if (e) {
                say("mmustress: cpu %d round %d create %d: errno %d\n", cpu, r, i, e);
                fails++;
                break;
            }
            made++;
        }
        for (int i = 0; i < made; i++)
            sem_post(&gate[cpu]);
        for (int i = 0; i < made; i++) {
            int e = pthread_join(t[i], NULL);
            if (e) {
                say("mmustress: cpu %d round %d join %d: errno %d\n", cpu, r, i, e);
                fails++;
            }
        }
        say("mmustress: cpu %d round %d: %d threads ok\n", cpu, r, made, 0);
        n = n * 2 > MAXT ? MAXT : n * 2;
    }
    return NULL;
}

int main(void)
{
    pthread_t s[CPUS];
    shm_unlink("/mmustress");
    shmfd = shm_open("/mmustress", O_RDWR | O_CREAT, 0600);
    if (shmfd < 0 || ftruncate(shmfd, SHMLEN) != 0) {
        perror("mmustress: shm");
        return 1;
    }
    printf("mmustress: start\n");
    fflush(stdout);
    for (int c = 0; c < CPUS; c++)
        sem_init(&gate[c], 0, 0);
    for (int c = 0; c < CPUS; c++)
        pthread_create(&s[c], NULL, spawner, (void *)(intptr_t)c);
    for (int c = 0; c < CPUS; c++)
        pthread_join(s[c], NULL);
    shm_unlink("/mmustress");
    printf("mmustress: %s\n", fails ? "FAIL" : "passed");
    return fails != 0;
}

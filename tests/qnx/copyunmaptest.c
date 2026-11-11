/* A message copy racing an munmap() of its buffer (review item IPC-2).
 * A server thread receives 64 KiB messages into a buffer that another of
 * its threads unmaps and maps again in a loop; a client sends a pattern
 * (0xa5) as fast as it can. Right after each munmap() the unmapping
 * thread maps a detector region at its own fixed address, which takes the
 * frames just freed (the free stack is last in, first out), and looks for
 * the pattern in it. The detector is never a receive buffer: the pattern
 * there means a copy went on writing a frame after the munmap() freed it,
 * into memory that was someone else's by then. A receive may fail with
 * EFAULT (its buffer unmapped) and the client's send with ESRVRFAULT,
 * which is fine. The race's window is one page's copy after its
 * translation, which the kernel's zeroing of the reused frame usually
 * covers: a stress test, it doesn't reproduce the bug on demand (log
 * entry 0124). Prints "copyunmaptest: passed" when no freed frame was
 * written. */
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define BUF (64 * 1024)
#define SECS 4
#define PATTERN 0xa5

static int chid;
static char *buf;
static volatile int stop;
static volatile long receives, faults, rounds, hits;

static double now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

static void *receiver(void *a)
{
    (void)a;
    while (!stop) {
        int rcvid = MsgReceive(chid, buf, BUF, NULL);
        if (rcvid > 0) {
            MsgReply(rcvid, 0, NULL, 0);
            receives++;
        } else if (rcvid == -1) {
            faults++;
        }
    }
    return NULL;
}

static void *unmapper(void *a)
{
    unsigned char *r = a;
    while (!stop) {
        /* The detector's own frames first, then the buffer's: freed last,
         * the buffer's are on top of the free stack, and the detector,
         * mapped again at its own address, gets them. */
        munmap(r, BUF);
        munmap(buf, BUF);
        if (mmap(r, BUF, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, NOFD, 0) == r) {
            /* time for a copy still at work to land */
            for (volatile int i = 0; i < 20000; i++) {
            }
            if (memchr(r, PATTERN, BUF)) {
                hits++;
            }
        }
        mmap(buf, BUF, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, NOFD, 0);
        rounds++;
    }
    return NULL;
}

int main(void)
{
    chid = ChannelCreate(0);
    buf = mmap(NULL, BUF, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    pid_t server = getpid();
    pid_t client = fork();
    if (client == 0) {
        int coid = ConnectAttach(0, server, chid, _NTO_SIDE_CHANNEL, 0);
        char *msg = malloc(BUF);
        memset(msg, PATTERN, BUF);
        double end = now() + SECS;
        while (now() < end) {
            MsgSend(coid, msg, BUF, NULL, 0);
        }
        _exit(0);
    }
    pthread_t r, u;
    pthread_create(&r, NULL, receiver, NULL);
    /* The detector: a region of its own, never a receive buffer. */
    void *detector = mmap(NULL, BUF, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    pthread_create(&u, NULL, unmapper, detector);
    int st;
    waitpid(client, &st, 0);
    stop = 1;
    pthread_join(u, NULL);
    pthread_cancel(r);
    printf("copyunmaptest: %ld messages received, %ld receives failed, %ld unmaps, %ld with the pattern in freed frames\n",
           receives, faults, rounds, hits);
    printf("copyunmaptest: %s no freed frame written by a copy\n", hits == 0 ? "ok" : "FAIL");
    printf("copyunmaptest: %s\n", hits == 0 ? "passed" : "FAILED");
    return hits != 0;
}

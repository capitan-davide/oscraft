/* Micro-benchmarks of kernel and process-manager operations, to compare
 * OsCraft with QNX on the same hardware: prints the time per operation
 * of each case, in microseconds. `bench exit` just exits (the program
 * spawn() starts); `bench open` times the steps of an open(), `bench
 * spawn` spawn() only. */
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procmsg.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static double now_us(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e6 + ts.tv_nsec / 1e3;
}

static void report(const char *what, int n, double t0)
{
    printf("bench: %-22s %10.2f us\n", what, (now_us() - t0) / n);
}

static int chid;

static void *server(void *arg)
{
    (void)arg;
    char buf[64];
    for (;;) {
        int rcvid = MsgReceive(chid, buf, sizeof buf, NULL);
        if (rcvid > 0)
            MsgReply(rcvid, 0, buf, 4);
    }
    return NULL;
}

/* The steps of an open(): a connection to procnto, a path lookup with
 * stat() (no file descriptor), and the whole open() + close(). */
static void open_steps(void)
{
    int n = 5000;
    double t0 = now_us();
    for (int i = 0; i < n; i++)
        ConnectDetach(ConnectAttach(0, PROCMGR_PID, PROCMGR_CHID, _NTO_SIDE_CHANNEL, 0));
    report("ConnectAttach+Detach", n, t0);

    struct stat st;
    t0 = now_us();
    for (int i = 0; i < n; i++)
        stat("/proc/boot/bench", &st);
    report("stat", n, t0);

    t0 = now_us();
    for (int i = 0; i < n; i++)
        close(open("/proc/boot/bench", O_RDONLY));
    report("open+close", n, t0);

    t0 = now_us();
    for (int i = 0; i < n; i++)
        close(open("/dev/null", O_RDONLY));
    report("open+close /dev/null", n, t0);
}

int main(int argc, char **argv)
{
    if (argc > 1 && !strcmp(argv[1], "exit"))
        return 0;
    if (argc > 1 && !strcmp(argv[1], "open")) {
        open_steps();
        return 0;
    }
    char *args[] = {argv[0], "exit", NULL};
    if (argc > 1 && !strcmp(argv[1], "spawn"))
        goto spawn;
    /* `bench msg`: the message round trip only (to trace it). */
    int only_msg = argc > 1 && !strcmp(argv[1], "msg");
    int n;
    double t0;

    if (!only_msg) {
        n = 200000;
        t0 = now_us();
        for (int i = 0; i < n; i++)
            sched_yield();
        report("sched_yield", n, t0);

        n = 20000;
        t0 = now_us();
        for (int i = 0; i < n; i++)
            getppid();
        report("getppid (to procnto)", n, t0);
    }

    chid = ChannelCreate(0);
    pthread_t t;
    pthread_create(&t, NULL, server, NULL);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    char msg[4] = "ping", reply[4];
    n = 50000;
    t0 = now_us();
    for (int i = 0; i < n; i++)
        MsgSend(coid, msg, sizeof msg, reply, sizeof reply);
    report("MsgSend round trip", n, t0);
    if (only_msg)
        return 0;

    n = 20000;
    t0 = now_us();
    for (int i = 0; i < n; i++) {
        void *p = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        *(volatile char *)p = 1;
        munmap(p, 4096);
    }
    report("mmap+touch+munmap", n, t0);

    n = 5000;
    t0 = now_us();
    for (int i = 0; i < n; i++)
        close(open("/proc/boot/bench", O_RDONLY));
    report("open+close", n, t0);

    n = 200;
    t0 = now_us();
    for (int i = 0; i < n; i++) {
        pid_t pid = fork();
        if (pid == 0)
            _exit(0);
        waitpid(pid, NULL, 0);
    }
    report("fork+exit+wait", n, t0);

spawn:
    n = 200;
    t0 = now_us();
    for (int i = 0; i < n; i++) {
        pid_t pid;
        if (posix_spawn(&pid, argv[0], NULL, NULL, args, environ) == 0)
            waitpid(pid, NULL, 0);
    }
    report("spawn+exit+wait", n, t0);
    return 0;
}

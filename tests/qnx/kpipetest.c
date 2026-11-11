/* Kernel pipes (PipeOpen() & co.) through QNX's kpipe manager, which this
 * test starts and stops again (pipe() goes to it meanwhile):
 * - a pipe holds 64 KB; non-blocking writes take what fits (EAGAIN if
 *   nothing), but up to the atomic size (5120) all or nothing;
 * - an empty pipe: EAGAIN if non-blocking; end of file once the writers
 *   are gone; a write without readers is SIGPIPE and EPIPE; no lseek()
 *   (ESPIPE);
 * - a blocked reader is woken by a write, a blocked writer by a read;
 * - poll() hears of data and of room (the kernel's notifications);
 * - dup()ed and inherited descriptors are the same pipe;
 * - data goes through in order, across processes;
 * - a read() into a buffer that faults is EFAULT and leaves the data in the
 *   pipe, even when part of it was copied (procnto; review IPC-21).
 * Prints one "ok"/"FAIL" line per check and "kpipetest: passed". */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static volatile int sigpipes;

static void check(const char *what, int ok)
{
    printf("kpipetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void on_sigpipe(int sig)
{
    (void)sig;
    sigpipes++;
}

/* The pid of the running kpipe, or -1. */
static pid_t kpipe_pid(void)
{
    DIR *d = opendir("/proc");
    struct dirent *e;
    pid_t found = -1;
    while (d && (e = readdir(d))) {
        char path[64], name[64] = "";
        snprintf(path, sizeof path, "/proc/%s/exefile", e->d_name);
        int fd = open(path, O_RDONLY);
        if (fd < 0) {
            continue;
        }
        int n = read(fd, name, sizeof name - 1);
        close(fd);
        name[n > 0 ? n : 0] = 0;
        if (strstr(name, "kpipe")) {
            found = atoi(e->d_name);
        }
    }
    if (d) {
        closedir(d);
    }
    return found;
}

int main(void)
{
    static char buf[200000];
    memset(buf, 'k', sizeof buf);
    system("kpipe");
    pid_t kp = kpipe_pid();
    check("kpipe runs", kp > 0);

    int p[2];
    check("pipe()", pipe(p) == 0);
    struct stat st;
    fstat(p[0], &st);
    check("fstat(): a FIFO (kpipe's)", S_ISFIFO(st.st_mode) && (st.st_mode & 0777) == 0777);

    fcntl(p[1], F_SETFL, O_NONBLOCK);
    int total = 0, n;
    while ((n = write(p[1], buf, 4096)) > 0) {
        total += n;
    }
    check("it holds 64 KB, then EAGAIN", total == 65536 && errno == EAGAIN);
    read(p[0], buf, 3000);
    errno = 0;
    check("an atomic write (4000) into 3000 free: EAGAIN", write(p[1], buf, 4000) == -1 && errno == EAGAIN);
    check("a larger write (10000) takes the 3000 that fit", write(p[1], buf, 10000) == 3000);
    struct pollfd pf = {p[1], POLLOUT, 0};
    check("poll(): a full pipe isn't writable", poll(&pf, 1, 0) == 0);
    read(p[0], buf, 6000);
    check("6000 read: writable again", poll(&pf, 1, 0) == 1);
    fcntl(p[0], F_SETFL, O_NONBLOCK);
    while (read(p[0], buf, sizeof buf) > 0) {
    }
    errno = 0;
    check("an empty pipe, non-blocking: EAGAIN", read(p[0], buf, 10) == -1 && errno == EAGAIN);
    errno = 0;
    check("lseek(): ESPIPE", lseek(p[0], 0, SEEK_SET) == -1 && errno == ESPIPE);
    close(p[1]);
    usleep(200000);
    check("the writer gone: end of file", read(p[0], buf, 10) == 0);
    close(p[0]);

    signal(SIGPIPE, on_sigpipe);
    pipe(p);
    close(p[0]);
    usleep(200000);
    errno = 0;
    check("a write without readers: EPIPE", write(p[1], "x", 1) == -1 && errno == EPIPE);
    check("... and SIGPIPE", sigpipes == 1);
    close(p[1]);

    /* A reader blocked until a child writes; the child blocked until the
     * parent reads. */
    pipe(p);
    pid_t pid = fork();
    if (pid == 0) {
        close(p[0]);
        usleep(200000);
        write(p[1], "hello", 5);
        /* Fill the pipe, then one more: blocks until the parent reads. */
        int w = 0;
        while (w < 65536 + 100) {
            int k = write(p[1], buf, 4096);
            if (k <= 0) {
                break;
            }
            w += k;
        }
        _exit(w >= 65536 + 100 ? 0 : 1);
    }
    close(p[1]);
    char got[8] = "";
    check("a blocked read woken by the child's write", read(p[0], got, 5) == 5 && !strcmp(got, "hello"));
    usleep(300000);
    long all = 0;
    while ((n = read(p[0], buf, sizeof buf)) > 0) {
        all += n;
    }
    int status = -1;
    waitpid(pid, &status, 0);
    check("the child's blocked write went on as the pipe emptied",
          WIFEXITED(status) && WEXITSTATUS(status) == 0 && all >= 65536);
    close(p[0]);

    /* poll() for input across processes, a dup()ed end, an inherited one. */
    pipe(p);
    int d = dup(p[1]);
    pid = fork();
    if (pid == 0) {
        close(p[0]);
        close(p[1]);
        usleep(200000);
        write(d, "dup", 3);
        _exit(0);
    }
    close(d);
    close(p[1]);
    struct pollfd in = {p[0], POLLIN, 0};
    int r = poll(&in, 1, 3000);
    check("poll() wakes when the child writes (through a dup)", r == 1 && (in.revents & POLLIN));
    memset(got, 0, sizeof got);
    errno = 0;
    n = read(p[0], got, 3);
    if (n != 3) {
        printf("kpipetest:   read %d errno %d revents %#x\n", n, errno, in.revents);
    }
    check("the data", n == 3 && !strcmp(got, "dup"));
    waitpid(pid, NULL, 0);
    in.revents = 0;
    r = poll(&in, 1, 3000);
    check("poll() hears of the last writer's end", r == 1 && (in.revents & (POLLIN | POLLHUP)));
    check("then end of file", read(p[0], got, 1) == 0);
    close(p[0]);

    /* 5 MB through, in order. */
    pipe(p);
    pid = fork();
    if (pid == 0) {
        close(p[0]);
        for (int i = 0; i < 1000; i++) {
            memset(buf, 'a' + i % 26, 5000);
            write(p[1], buf, 5000);
        }
        _exit(0);
    }
    close(p[1]);
    long sum = 0, bad = 0;
    char in5[5000];
    for (int i = 0; i < 1000; i++) {
        int k = 0;
        while (k < 5000 && (n = read(p[0], in5 + k, 5000 - k)) > 0) {
            k += n;
        }
        sum += k;
        bad += k != 5000 || in5[0] != 'a' + i % 26 || in5[4999] != 'a' + i % 26;
    }
    waitpid(pid, NULL, 0);
    check("5 MB in 5000-byte (atomic) writes, in order", sum == 5000000 && bad == 0);
    close(p[0]);

    pipe(p);
    fcntl(p[0], F_SETFL, O_NONBLOCK);
    char data[6000];
    for (int i = 0; i < 6000; i++) {
        data[i] = 'a' + i % 26;
    }
    char *area = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    mprotect(area, 8192, PROT_NONE);
    write(p[1], data, 6000);
    errno = 0;
    r = read(p[0], area, 100);
    check("a read() into an unmapped buffer: EFAULT", r == -1 && errno == EFAULT);
    mprotect(area, 4096, PROT_READ | PROT_WRITE);
    errno = 0;
    r = read(p[0], area, 8192);
    check("a read() into a half-mapped one: EFAULT", r == -1 && errno == EFAULT);
    char back[8000];
    r = read(p[0], back, sizeof back);
    check("the data stays in the pipe, all of it, in order", r == 6000 && memcmp(back, data, 6000) == 0);
    close(p[0]);
    close(p[1]);
    munmap(area, 8192);

    if (kp > 0) {
        kill(kp, SIGTERM);
        for (int i = 0; i < 50 && kpipe_pid() > 0; i++) {
            usleep(20000);
        }
    }
    printf("kpipetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

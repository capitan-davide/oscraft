/* A fork() that runs out of memory (review items CORE-4, PROCMGR-5 and
 * CORE-29): with more than half of the free memory mapped (and backed:
 * QNX maps fully at map time), fork() can't copy it and fails with
 * ENOMEM. Then: the copied frames must be free again, there must be no
 * half-made child (wait() says ECHILD, /proc lists no new child of ours;
 * a boot service may start meanwhile), and the parent must still work.
 * Before the fix the copy took the kernel's last frames and its next
 * allocation panicked. Prints one "ok"/"FAIL" line per check and
 * "forkfail: passed". */
#include <devctl.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/procfs.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("forkfail: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* free memory, as pidin info reads it */
static long long free_mem(void)
{
    struct stat st;
    return stat("/proc", &st) == 0 ? (long long)st.st_size : -1;
}

/* the pids /proc lists, as a sorted array; returns how many */
static int processes(int *pids, int max)
{
    DIR *d = opendir("/proc");
    struct dirent *e;
    int n = 0;
    if (!d)
        return -1;
    while ((e = readdir(d)) != NULL)
        if (e->d_name[0] >= '1' && e->d_name[0] <= '9' && n < max)
            pids[n++] = atoi(e->d_name);
    closedir(d);
    for (int i = 1; i < n; i++)
        for (int j = i; j > 0 && pids[j - 1] > pids[j]; j--) {
            int t = pids[j];
            pids[j] = pids[j - 1];
            pids[j - 1] = t;
        }
    return n;
}

/* the parent of process pid, or -1 */
static int parent_of(int pid)
{
    char path[64];
    procfs_info info;
    snprintf(path, sizeof path, "/proc/%d/as", pid);
    int fd = open(path, O_RDONLY);
    if (fd < 0)
        return -1;
    int r = devctl(fd, DCMD_PROC_INFO, &info, sizeof info, NULL);
    close(fd);
    return r == 0 ? (int)info.parent : -1;
}

/* pids in b that are not in a (both sorted) and are our children (a
 * boot service may start meanwhile): printed; returns how many */
static int new_children(const int *a, int na, const int *b, int nb)
{
    int n = 0;
    for (int i = 0, j = 0; j < nb; j++) {
        while (i < na && a[i] < b[j])
            i++;
        if (i >= na || a[i] != b[j]) {
            int parent = parent_of(b[j]);
            printf("forkfail: new process %d, parent %d%s\n", b[j], parent,
                   parent == getpid() ? " (ours!)" : "");
            n += parent == getpid();
        }
    }
    return n;
}

/* Boot services (io-sock, sshd, ...) may still be starting, and take
 * memory: wait until the process count has been steady for a while. */
static void quiet_boot(void)
{
    int pids[256], last = -1, steady = 0;
    for (int i = 0; i < 60 && steady < 4; i++) {
        int n = processes(pids, 256);
        steady = n == last ? steady + 1 : 0;
        last = n;
        usleep(500000);
    }
}

int main(void)
{
    static int pids0[256], pids1[256];
    quiet_boot();
    long long free0 = free_mem();
    int procs0 = processes(pids0, 256);
    size_t size = (size_t)(free0 * 6 / 10) & ~(size_t)0xfff;
    printf("forkfail: free %lld MiB, %d processes, mapping %zu MiB\n",
           free0 >> 20, procs0, size >> 20);
    char *big = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    check("map more than half of memory", big != MAP_FAILED);
    if (big == MAP_FAILED)
        return 1;
    memset(big, 0x5a, size);
    long long free1 = free_mem();
    check("it is backed: free memory down by about its size",
          free0 - free1 > (long long)size - (16 << 20) && free0 - free1 < (long long)size + (64 << 20));
    int errs[3] = { 0, 0, 0 };
    for (int i = 0; i < 3; i++) {
        pid_t pid = fork();
        if (pid == 0)
            _exit(0);
        errs[i] = pid < 0 ? errno : 0;
        if (pid > 0)
            waitpid(pid, NULL, 0);
    }
    printf("forkfail: fork errors %d %d %d (ENOMEM %d, EAGAIN %d)\n", errs[0], errs[1], errs[2], ENOMEM, EAGAIN);
    check("fork fails three times", errs[0] && errs[1] && errs[2]);
    check("with ENOMEM", errs[0] == ENOMEM && errs[1] == ENOMEM && errs[2] == ENOMEM);
    long long free2 = free_mem();
    printf("forkfail: free after the forks %lld MiB (before %lld MiB)\n", free2 >> 20, free1 >> 20);
    check("the failed forks' frames are free again", free2 > free1 - (32 << 20));
    check("wait: ECHILD, no half-made child", waitpid(-1, NULL, 0) < 0 && errno == ECHILD);
    int procs1 = processes(pids1, 256);
    printf("forkfail: %d processes (before %d)\n", procs1, procs0);
    check("no new child in /proc", new_children(pids0, procs0, pids1, procs1) == 0);
    check("the mapping is intact", big[0] == 0x5a && big[size - 1] == 0x5a);
    munmap(big, size);
    long long free3 = free_mem();
    check("free memory back after munmap", free3 - free2 > (long long)size - (32 << 20));
    pid_t pid = fork();
    if (pid == 0)
        _exit(7);
    int status = -1;
    check("a fork works again", pid > 0 && waitpid(pid, &status, 0) == pid && WEXITSTATUS(status) == 7);
    printf("forkfail: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

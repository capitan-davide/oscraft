/* fork() with copy-on-write pages: after the fork, parent and child each
 * see only their own writes (globals, heap, stack), including a page three
 * processes share, and pages made read-only and writable again. The child
 * has the parent's file descriptors but not its side channel connections
 * (procnto's stay). Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static int global = 1;

static void check(const char *what, int ok)
{
    printf("forktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

/* Run `child` in a child process; its exit status is the result. */
static int in_child(int (*child)(void *), void *arg)
{
    pid_t pid = fork();
    if (pid == 0)
        _exit(child(arg));
    int status = -1;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 99;
}

static int child_writes(void *arg)
{
    char *heap = arg;
    int stack = 7;
    global = 2;
    memset(heap, 'c', 65536);
    stack = 8;
    return global == 2 && heap[100] == 'c' && stack == 8 ? 0 : 1;
}

static int child_waits(void *arg)
{
    char *heap = arg;
    usleep(200 * 1000); /* the parent writes meanwhile */
    return global == 1 && heap[100] == 'p' ? 0 : 1;
}

static int grandchild(void *arg)
{
    char *heap = arg;
    heap[0] = 'g';
    return heap[1] == 'p' ? 0 : 1;
}

static int side_coid, null_fd;

static int child_connections(void *arg)
{
    (void)arg;
    struct _server_info si;
    errno = 0;
    int side_gone = MsgSendPulse(side_coid, 10, 1, 0) == -1 && errno == EBADF;
    int fd_kept = ConnectServerInfo(0, null_fd, &si) == null_fd;
    int procnto_kept = ConnectServerInfo(0, _NTO_SIDE_CHANNEL, &si) == _NTO_SIDE_CHANNEL;
    return side_gone && fd_kept && procnto_kept ? 0 : 1;
}

static int child_forks(void *arg)
{
    char *heap = arg;
    int r = in_child(grandchild, heap);
    return r == 0 && heap[0] == 'p' ? 0 : 1;
}

static int child_protects(void *arg)
{
    char *page = arg;
    if (mprotect(page, 4096, PROT_READ) || mprotect(page, 4096, PROT_READ | PROT_WRITE))
        return 2;
    page[0] = 'x';
    return 0;
}

int main(void)
{
    char *heap = malloc(65536);
    memset(heap, 'p', 65536);
    int stack = 5;

    check("child's writes are its own", in_child(child_writes, heap) == 0);
    check("parent keeps its pages", global == 1 && heap[100] == 'p' && stack == 5);

    pid_t pid = fork();
    if (pid == 0)
        _exit(child_waits(heap));
    global = 3;
    memset(heap, 'q', 65536);
    int status = -1;
    waitpid(pid, &status, 0);
    check("parent's writes are its own", WIFEXITED(status) && WEXITSTATUS(status) == 0);
    global = 1;
    memset(heap, 'p', 65536);

    check("a page shared by three processes", in_child(child_forks, heap) == 0 && heap[0] == 'p');

    char *page = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    page[0] = 'm';
    check("mprotect of a shared page", in_child(child_protects, page) == 0 && page[0] == 'm');

    /* A connection to our own channel: the child waiting on it would
     * deadlock the fork. */
    int chid = ChannelCreate(0);
    side_coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    null_fd = open("/dev/null", O_RDONLY);
    check("fds inherited, side channels not", in_child(child_connections, NULL) == 0);
    ConnectDetach(side_coid);
    ChannelDestroy(chid);

    int n = 0;
    for (int i = 0; i < 100; i++)
        n += in_child(child_writes, heap) == 0;
    check("100 forks", n == 100 && heap[100] == 'p');

    printf("forktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

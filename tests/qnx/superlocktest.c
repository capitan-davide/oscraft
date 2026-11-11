/* QNX 8's "superlocked" memory: everything is backed when it's mapped, so
 * nothing is faulted in later. Checks, with posix_mem_offset() (a mapped
 * page has a physical address): the lowest page of the main thread's
 * stack, never touched; a fresh anonymous mapping, never touched (even
 * with MAP_LAZY, ignored); and, right after fork(), a page the child has
 * not written is already its own copy (no copy-on-write). Prints one
 * "ok"/"FAIL" line per case. */
#include <stdio.h>
#include <sys/mman.h>
#include <sys/storage.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("superlocktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

/* The physical address of `p`, or 0 if it has none. */
static off64_t phys(const void *p)
{
    off64_t off = 0;
    size_t len = 0;
    if (posix_mem_offset64(p, 1, &off, &len, NULL) != 0)
        return 0;
    return off;
}

static char page_data[2 * 4096] __attribute__((aligned(4096))) = {1};

int main(void)
{
    struct _thread_local_storage *tls = __tls();
    char *bottom = (char *)tls->__stackaddr;
    check("the main stack's lowest page is backed", phys(bottom) != 0);

    char *anon = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    check("an untouched anonymous mapping is backed", anon != MAP_FAILED && phys(anon) != 0);
    char *lazy = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_LAZY, -1, 0);
    check("MAP_LAZY is ignored", lazy != MAP_FAILED && phys(lazy) != 0);

    off64_t parent = phys(page_data);
    int fd[2];
    pipe(fd);
    pid_t pid = fork();
    if (pid == 0) {
        off64_t mine = phys(page_data); /* read only: no write before */
        write(fd[1], &mine, sizeof mine);
        _exit(0);
    }
    off64_t child = 0;
    read(fd[0], &child, sizeof child);
    waitpid(pid, NULL, 0);
    check("fork() copies at once", parent != 0 && child != 0 && child != parent);

    printf("superlocktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

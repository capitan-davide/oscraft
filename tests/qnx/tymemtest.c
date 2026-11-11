/* Typed memory pools: what posix_typed_mem_get_info() reports and which
 * names open (review item PATHMEM-26), as procnto (measured):
 * - opened to allocate scattered (POSIX_TYPED_MEM_ALLOCATE): the pool's
 *   free memory, about all free memory for /memory;
 * - opened to allocate contiguous (POSIX_TYPED_MEM_ALLOCATE_CONTIG): the
 *   largest free run, no more than the scattered figure;
 * - opened for neither: 0;
 * - a pool opens by its own name, /below4G, which here holds all of
 *   /memory; QEMU's virt has its RAM from 1 GiB up, so the startup makes
 *   no /below1G there, and the name is ENOENT (a board with RAM below
 *   1 GiB has that pool, with less than /memory);
 * - a pool may be named by its path in the system page's tree,
 *   /memory/below4G, as by its own name; a name that isn't there: ENOENT.
 * The kernel used to report the largest contiguous run in every case and
 * refuse paths. Prints one "ok"/"FAIL" line per check and
 * "tymemtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("tymemtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* the pool's posix_tmi_length in MiB, or -1 (errno) if it doesn't open */
static long info(const char *pool, int tflag)
{
    int fd = posix_typed_mem_open(pool, O_RDWR, tflag);
    if (fd < 0) {
        return -1;
    }
    struct posix_typed_mem_info i;
    memset(&i, 0, sizeof i);
    int r = posix_typed_mem_get_info(fd, &i);
    close(fd);
    return r == 0 ? (long)(i.posix_tmi_length >> 20) : -2;
}

int main(void)
{
    char what[160];
    /* all free memory, as the memory manager reports it */
    struct posix_typed_mem_info all;
    memset(&all, 0, sizeof all);
    long sys_free = -1;
    int fd = posix_typed_mem_open("/memory", O_RDWR, POSIX_TYPED_MEM_ALLOCATE);
    if (fd >= 0) {
        posix_typed_mem_get_info(fd, &all);
        close(fd);
        sys_free = (long)(all.posix_tmi_length >> 20);
    }
    long scattered = info("/memory", POSIX_TYPED_MEM_ALLOCATE);
    long contig = info("/memory", POSIX_TYPED_MEM_ALLOCATE_CONTIG);
    long neither = info("/memory", 0);
    snprintf(what, sizeof what, "/memory, ALLOCATE: %ld MiB, most of memory", scattered);
    check(what, scattered > 1024);
    snprintf(what, sizeof what, "/memory, ALLOCATE_CONTIG: %ld MiB, at most the scattered figure", contig);
    check(what, contig > 0 && contig <= scattered);
    snprintf(what, sizeof what, "/memory, neither: %ld MiB", neither);
    check(what, neither == 0);
    long below4g = info("/below4G", POSIX_TYPED_MEM_ALLOCATE);
    /* measured a moment after /memory: free memory moves a little */
    snprintf(what, sizeof what, "/below4G, by its own name, ALLOCATE: %ld MiB, about /memory", below4g);
    check(what, below4g > 0 && below4g <= scattered + 16);
    errno = 0;
    long below1g = info("/below1G", POSIX_TYPED_MEM_ALLOCATE);
    snprintf(what, sizeof what, "/below1G, where RAM starts at 1 GiB: %ld errno %d (ENOENT %d)", below1g, errno, ENOENT);
    check(what, below1g == -1 && errno == ENOENT);
    long nested = info("/memory/below4G", POSIX_TYPED_MEM_ALLOCATE);
    snprintf(what, sizeof what, "/memory/below4G opens: %ld MiB", nested);
    check(what, nested > 0);
    errno = 0;
    long none = info("/memory/nosuchpool", POSIX_TYPED_MEM_ALLOCATE);
    check("/memory/nosuchpool: ENOENT", none == -1 && errno == ENOENT);
    (void)sys_free;
    printf("tymemtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

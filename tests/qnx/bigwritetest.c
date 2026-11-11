/* Long writes to procnto's files (issue #285), as procnto (measured): one
 * write() or pwrite() takes the whole request, whatever its size, to a
 * /dev/shmem file, /proc/<pid>/as and /dev/null, the data intact. The
 * kernel took 4 MiB less the message's header (4194288 bytes for a
 * write(), 4194280 for a pwrite()). A source buffer that faults part-way
 * is EFAULT, the file sized for all of it, every byte before the fault in
 * it and zeros after, the position where it was. Prints one "ok"/"FAIL" line per check
 * and "bigwritetest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define MB (1024 * 1024)

static int fails;
static char *buf;

static void check(const char *what, int ok)
{
    printf("bigwritetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* the n bytes at offset `at` of fd are buf's first n */
static int intact(int fd, size_t n, off_t at)
{
    char *b = malloc(n);
    int ok = pread(fd, b, n, at) == (ssize_t)n && memcmp(b, buf, n) == 0;
    free(b);
    return ok;
}

int main(void)
{
    buf = malloc(16 * MB);
    for (size_t i = 0; i < 16 * MB; i++)
        buf[i] = (char)(i * 7 + 3);

    int fd = open("/dev/shmem/bigwritetest", O_RDWR | O_CREAT | O_TRUNC, 0600);
    unlink("/dev/shmem/bigwritetest");
    check("write(4 MiB) to a /dev/shmem file: all of it", write(fd, buf, 4 * MB) == 4 * MB);
    check("and the data is intact", intact(fd, 4 * MB, 0));
    check("and the position moved by 4 MiB", lseek(fd, 0, SEEK_CUR) == 4 * MB);
    check("write(16 MiB): all of it", write(fd, buf, 16 * MB) == 16 * MB);
    check("and the data is intact", intact(fd, 16 * MB, 4 * MB));
    check("pwrite(16 MiB, at 4096): all of it", pwrite(fd, buf, 16 * MB, 4096) == 16 * MB);
    check("and the data is intact", intact(fd, 16 * MB, 4096));
    check("and the position stayed", lseek(fd, 0, SEEK_CUR) == 20 * MB);
    close(fd);

    /* 200 KiB mapped of a 1 MiB source buffer */
    char *b = mmap(NULL, 2 * MB, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    memcpy(b, buf, 200 * 1024);
    munmap(b + 200 * 1024, 2 * MB - 200 * 1024);
    fd = open("/dev/shmem/bigwritetest", O_RDWR | O_CREAT | O_TRUNC, 0600);
    unlink("/dev/shmem/bigwritetest");
    errno = 0;
    check("write(1 MiB) from 200 KiB of mapped buffer: EFAULT", write(fd, b, MB) == -1 && errno == EFAULT);
    check("the position where it was", lseek(fd, 0, SEEK_CUR) == 0);
    check("the file sized for all of it", lseek(fd, 0, SEEK_END) == MB);
    check("the 200 KiB before the fault in it", intact(fd, 200 * 1024, 0));
    char *r = calloc(1, MB);
    pread(fd, r, MB, 0);
    int zero = 1;
    for (size_t i = 200 * 1024; i < MB; i++)
        zero &= r[i] == 0;
    check("zeros after it", zero);
    close(fd);

    char *m = mmap(NULL, 8 * MB, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    fd = open("/proc/self/as", O_RDWR);
    check("pwrite(8 MiB) to /proc/self/as: all of it", pwrite(fd, buf, 8 * MB, (off_t)(uintptr_t)m) == 8 * MB);
    check("and the memory has it", memcmp(m, buf, 8 * MB) == 0);
    close(fd);

    fd = open("/dev/null", O_WRONLY);
    check("write(16 MiB) to /dev/null: all of it", write(fd, buf, 16 * MB) == 16 * MB);
    close(fd);

    printf("bigwritetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

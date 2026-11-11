/* Long reads of procnto's files (review item PATHMEM-14), as procnto
 * (measured): one read() copies the whole request from a /dev/shmem file
 * (up to its end), /dev/zero and /proc/<pid>/as; /proc/<pid>/as reads up
 * to the first unmapped byte, and starting at one is ESRVRFAULT; a
 * client buffer that isn't all mapped is EFAULT. The kernel returned at
 * most 64 KiB per read. Prints one "ok"/"FAIL" line per check and
 * "bigreadtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#define K 1024
#define BIG (1024 * K)

static int fails;

static void check(const char *what, int ok)
{
    printf("bigreadtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* every byte of b[0..n) is c */
static int all(const char *b, size_t n, char c)
{
    for (size_t i = 0; i < n; i++)
        if (b[i] != c)
            return 0;
    return 1;
}

int main(void)
{
    char *buf = malloc(BIG);

    memset(buf, 'x', BIG);
    int fd = open("/dev/shmem/bigreadtest", O_RDWR | O_CREAT | O_TRUNC, 0600);
    unlink("/dev/shmem/bigreadtest");
    check("write(1 MiB) to a /dev/shmem file: all of it", write(fd, buf, BIG) == BIG);
    memset(buf, 0, BIG);
    lseek(fd, 0, SEEK_SET);
    check("read(1 MiB) from it: all of it", read(fd, buf, BIG) == BIG && all(buf, BIG, 'x'));
    check("and the position moved by 1 MiB", lseek(fd, 0, SEEK_CUR) == BIG);
    check("pread(1 MiB) 100 KiB before its end: 100 KiB",
          pread(fd, buf, BIG, BIG - 100 * K) == 100 * K && all(buf, 100 * K, 'x'));
    close(fd);

    memset(buf, 1, BIG);
    fd = open("/dev/zero", O_RDONLY);
    check("read(1 MiB) from /dev/zero: all of it, zeros", read(fd, buf, BIG) == BIG && all(buf, BIG, 0));
    char *b = mmap(NULL, 128 * K, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    munmap(b + 64 * K, 64 * K);
    errno = 0;
    check("read(128 KiB) from /dev/zero into 64 KiB of mapped buffer: EFAULT",
          read(fd, b, 128 * K) == -1 && errno == EFAULT);
    close(fd);

    char *m = mmap(NULL, 512 * K, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    memset(m, 'y', 512 * K);
    off_t at = (off_t)(uintptr_t)m;
    fd = open("/proc/self/as", O_RDONLY);
    memset(buf, 0, BIG);
    check("pread(512 KiB) from /proc/self/as: all of it",
          pread(fd, buf, 512 * K, at) == 512 * K && all(buf, 512 * K, 'y'));
    munmap(m + 200 * K, 312 * K);
    lseek(fd, at, SEEK_SET);
    check("read(1 MiB) of 200 KiB mapped then a hole: 200 KiB",
          read(fd, buf, BIG) == 200 * K && all(buf, 200 * K, 'y'));
    check("and the position moved by 200 KiB", lseek(fd, 0, SEEK_CUR) == at + 200 * K);
    check("pread(100 KiB) of 50 KiB mapped then a hole: 50 KiB", pread(fd, buf, 100 * K, at + 150 * K) == 50 * K);
    errno = 0;
    check("pread() starting in the hole: ESRVRFAULT", pread(fd, buf, BIG, at + 300 * K) == -1 && errno == ESRVRFAULT);
    close(fd);

    fd = open("/dev/null", O_WRONLY);
    check("write(1 MiB) to /dev/null: all of it", write(fd, buf, BIG) == BIG);
    close(fd);

    printf("bigreadtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

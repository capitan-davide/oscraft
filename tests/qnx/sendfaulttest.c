/* A write() whose buffer faults part-way (issue #288), as procnto
 * (measured): procnto's process manager receives the first 2088 bytes of
 * a message. A fault within them fails the client's send (EFAULT, the
 * file untouched); past them the message arrives, the file is sized for
 * the whole write, the bytes before the fault are in it, and the write
 * fails with EFAULT, the position unmoved (procnto's copy may leave up to
 * 56 bytes just before the fault out, by the buffer's alignment: only the
 * bytes up to 64 before it are checked). OsCraft received up to 64 KiB,
 * so a fault in a write's first 64 KiB left the file empty. Prints
 * "sendfaulttest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define K 1024
#define PG 4096

static int fails;

static void check(const char *what, int ok)
{
    printf("sendfaulttest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* write(len) from a buffer that faults `k` bytes in; the file's size, and
 * whether its first k bytes are the buffer's. */
static void one(size_t len, size_t k, off_t want_size)
{
    size_t pages = (k + PG - 1) / PG + 1;
    char *area = mmap(NULL, pages * PG, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
    memset(area, 'x', pages * PG);
    mprotect(area + (pages - 1) * PG, PG, PROT_NONE);
    char *b = area + (pages - 1) * PG - k;
    int fd = open("/dev/shmem/sendfaulttest", O_RDWR | O_CREAT | O_TRUNC, 0600);
    unlink("/dev/shmem/sendfaulttest");
    errno = 0;
    ssize_t r = write(fd, b, len);
    int e = errno;
    struct stat st;
    fstat(fd, &st);
    int intact = 1;
    if (st.st_size > 0) {
        char c;
        size_t sure = k > 64 ? k - 64 : 0;
        for (size_t i = 0; i < sure; i += 509)
            intact &= pread(fd, &c, 1, i) == 1 && c == 'x';
        intact &= sure == 0 || (pread(fd, &c, 1, sure - 1) == 1 && c == 'x');
        intact &= (size_t)st.st_size == k || (pread(fd, &c, 1, k) == 1 && c == 0);
    }
    char what[160];
    snprintf(what, sizeof what, "write(%zu) faulting %zu bytes in: EFAULT, size %lld (got %zd/%d, size %lld)", len, k,
             (long long)want_size, r, r < 0 ? e : 0, (long long)st.st_size);
    check(what, r == -1 && e == EFAULT && st.st_size == want_size && intact && lseek(fd, 0, SEEK_CUR) == 0);
    close(fd);
    munmap(area, pages * PG);
}

int main(void)
{
    one(64 * K, 0, 0);
    one(64 * K, 2071, 0);
    one(64 * K, 2072, 64 * K);
    one(64 * K, 32 * K, 64 * K);
    one(128 * K, 96 * K, 128 * K);
    printf("sendfaulttest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

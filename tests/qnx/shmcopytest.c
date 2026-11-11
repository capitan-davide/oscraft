/* Private mappings of large shared-memory objects (review item
 * PATHMEM-5: the kernel used to build a buffer the size of the mapping
 * to copy through, which a mapping of most of memory made a panic):
 * - an object of 45% of free memory, written through a shared mapping,
 *   mapped MAP_PRIVATE: the copy reads the object's bytes, a write to it
 *   doesn't reach the object, and the memory comes back on unmap;
 * - a sparse object of 95% of free memory (ftruncate only), mapped
 *   MAP_PRIVATE: the mapping is made or refused with ENOMEM, and the
 *   process can still map memory after.
 * Prints one "ok"/"FAIL" line per check and "shmcopytest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("shmcopytest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long long free_mem(void)
{
    struct stat st;
    return stat("/proc", &st) == 0 ? (long long)st.st_size : -1;
}

/* Boot services may still be starting: wait until the free memory has
 * been steady for a while. */
static long long quiet_free(void)
{
    long long last = -1, now;
    int steady = 0;
    for (int i = 0; i < 60 && steady < 4; i++) {
        now = free_mem();
        steady = now >> 20 == last >> 20 ? steady + 1 : 0;
        last = now;
        usleep(500000);
    }
    return last;
}

int main(void)
{
    long long free0 = quiet_free();
    size_t len = (size_t)(free0 * 45 / 100) & ~(size_t)0xfffff;
    printf("shmcopytest: free %lld MiB, object %zu MiB\n", free0 >> 20, len >> 20);
    int fd = shm_open("/shmcopy", O_RDWR | O_CREAT, 0600);
    check("ftruncate", fd >= 0 && ftruncate(fd, len) == 0);
    char *shared = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("a shared mapping", shared != MAP_FAILED);
    if (shared != MAP_FAILED) {
        memset(shared, 0x5a, len);
        shared[0] = 1;
        shared[len - 1] = 2;
    }
    errno = 0;
    char *priv = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    check("a private mapping of 45% of memory", priv != MAP_FAILED);
    if (priv != MAP_FAILED) {
        check("it reads the object's bytes", priv[0] == 1 && priv[len / 2] == 0x5a && priv[len - 1] == 2);
        priv[len / 2] = 7;
        check("a write to it doesn't reach the object", shared[len / 2] == 0x5a);
        munmap(priv, len);
    }
    long long free1 = free_mem();
    check("the copy's memory is back after munmap", free1 > free0 - (long long)len - (64 << 20));
    if (shared != MAP_FAILED) {
        munmap(shared, len);
    }
    close(fd);
    shm_unlink("/shmcopy");

    long long free2 = free_mem();
    size_t big = (size_t)(free2 * 95 / 100) & ~(size_t)0xfffff;
    fd = shm_open("/shmcopy2", O_RDWR | O_CREAT, 0600);
    int r = ftruncate(fd, big);
    printf("shmcopytest: sparse object %zu MiB: ftruncate -> %d\n", big >> 20, r);
    errno = 0;
    priv = mmap(NULL, big, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    int err = errno;
    printf("shmcopytest: its private mapping -> %s errno %d\n", priv == MAP_FAILED ? "MAP_FAILED" : "ok", priv == MAP_FAILED ? err : 0);
    check("a private mapping of a sparse 95% object: made or ENOMEM", priv != MAP_FAILED || err == ENOMEM);
    if (priv != MAP_FAILED) {
        priv[big - 1] = 2;
        munmap(priv, big);
    }
    close(fd);
    shm_unlink("/shmcopy2");
    char *small = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_ANON | MAP_PRIVATE, -1, 0);
    check("the kernel is still there: a 1 MiB mapping after", small != MAP_FAILED);
    if (small != MAP_FAILED) {
        munmap(small, 1 << 20);
    }
    printf("shmcopytest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

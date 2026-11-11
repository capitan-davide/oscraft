/* shm_ctl() that can't have the memory (review item PATHMEM-15), as
 * procnto (measured): asking a typed memory pool for more than it has is
 * ENOMEM before anything is taken, the object's size and flags as they
 * were, the pool's memory all there, and a later shm_ctl() works; a
 * scattered allocation from a pool is quick. The kernel took the pool's
 * memory a page at a time, scanning the pool from its start for each
 * with interrupts masked, and grew the object as it went: asking for
 * more than the pool had froze the system. Prints one "ok"/"FAIL" line per
 * check and "shmfailtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("shmfailtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long long size_of(int fd)
{
    struct stat st;
    fstat(fd, &st);
    return st.st_size;
}

static long long pool_free(int t)
{
    struct posix_typed_mem_info i;
    posix_typed_mem_get_info(t, &i);
    return (long long)i.posix_tmi_length;
}

static double now(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec + ts.tv_nsec / 1e9;
}

/* asks pool `t` for more than it has, on an object that has `first` */
static void too_much(const char *what, int t, int phys, long long first)
{
    char line[160];
    int f = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    if (first)
        shm_ctl(f, SHMCTL_ANON | SHMCTL_TYMEM | phys, t, first);
    long long before = pool_free(t);
    errno = 0;
    int r = shm_ctl(f, SHMCTL_ANON | SHMCTL_TYMEM | phys, t, before + first + (64LL << 20));
    snprintf(line, sizeof line, "%s: more than the pool has: ENOMEM", what);
    check(line, r == -1 && errno == ENOMEM);
    snprintf(line, sizeof line, "%s: the object as it was", what);
    check(line, size_of(f) == first && shm_ctl(f, SHMCTL_GET_FLAGS, 0, 0) == (first ? SHMCTL_ANON | phys : 0));
    snprintf(line, sizeof line, "%s: the pool's memory all there", what);
    check(line, pool_free(t) >= before - (1 << 20));
    snprintf(line, sizeof line, "%s: then 64 KiB more", what);
    check(line, shm_ctl(f, SHMCTL_ANON | SHMCTL_TYMEM | phys, t, first + 65536) == 0 && size_of(f) == first + 65536);
    close(f);
}

int main(void)
{
    int ta = posix_typed_mem_open("/sysram", O_RDWR, POSIX_TYPED_MEM_ALLOCATE);
    int tc = posix_typed_mem_open("/sysram", O_RDWR, POSIX_TYPED_MEM_ALLOCATE_CONTIG);
    too_much("scattered", ta, 0, 0);
    too_much("scattered, 16 KiB there", ta, 0, 16384);
    too_much("contiguous", tc, SHMCTL_PHYS, 0);

    int f = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    double t0 = now();
    check("256 MiB scattered from a pool", shm_ctl(f, SHMCTL_ANON | SHMCTL_TYMEM, ta, 256 << 20) == 0);
    check("in under 10 s", now() - t0 < 10);
    char *p = mmap(NULL, 256 << 20, PROT_READ, MAP_SHARED, f, 0);
    check("zeroed", p != MAP_FAILED && p[0] == 0 && p[(256 << 20) - 1] == 0);
    close(f);

    f = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    errno = 0;
    check("SHMCTL_ANON of 1 TiB: ENOMEM, the object as it was",
          shm_ctl(f, SHMCTL_ANON, 0, 1LL << 40) == -1 && errno == ENOMEM && size_of(f) == 0 &&
              shm_ctl(f, SHMCTL_GET_FLAGS, 0, 0) == 0);
    close(f);

    printf("shmfailtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

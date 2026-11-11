/* mmap() and msync() flag checks (review items PATHMEM-11, PATHMEM-12),
 * as procnto (measured): MAP_ANON or MAP_PHYS with a descriptor is
 * EINVAL (MAP_PHYS | MAP_PRIVATE with NOFD, and MAP_PHYS | MAP_ANON, map),
 * and msync() takes at most one of MS_SYNC, MS_ASYNC and MS_SYNC_FULL,
 * MS_CACHE_ONLY with any, no unknown flag. The kernel mapped anonymous or
 * physical memory asked with a descriptor. Prints one "ok"/"FAIL" line per
 * check and "mmapflagtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("mmapflagtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* 0 if mapped (and unmapped again), else the errno */
static int m(int prot, int flags, int fd)
{
    errno = 0;
    void *p = mmap(NULL, 4096, prot, flags, fd, 0);
    if (p == MAP_FAILED)
        return errno;
    munmap(p, 4096);
    return 0;
}

static int s(void *p, int flags)
{
    errno = 0;
    return msync(p, 4096, flags) == 0 ? 0 : errno;
}

int main(void)
{
    int fd = shm_open("/mmapflagtest", O_RDWR | O_CREAT, 0600);
    shm_unlink("/mmapflagtest");
    ftruncate(fd, 4096);
    int rw = PROT_READ | PROT_WRITE;
    check("MAP_ANON | MAP_PRIVATE, NOFD: mapped", m(rw, MAP_ANON | MAP_PRIVATE, NOFD) == 0);
    check("MAP_ANON | MAP_PRIVATE with a descriptor: EINVAL", m(rw, MAP_ANON | MAP_PRIVATE, fd) == EINVAL);
    check("MAP_ANON | MAP_SHARED with a descriptor: EINVAL", m(rw, MAP_ANON | MAP_SHARED, fd) == EINVAL);
    check("MAP_PHYS | MAP_SHARED, NOFD: mapped", m(PROT_READ, MAP_PHYS | MAP_SHARED, NOFD) == 0);
    check("MAP_PHYS | MAP_PRIVATE, NOFD: mapped", m(PROT_READ, MAP_PHYS | MAP_PRIVATE, NOFD) == 0);
    check("MAP_PHYS | MAP_SHARED with a descriptor: EINVAL", m(PROT_READ, MAP_PHYS | MAP_SHARED, fd) == EINVAL);
    check("MAP_PHYS | MAP_ANON | MAP_PRIVATE: mapped", m(rw, MAP_PHYS | MAP_ANON | MAP_PRIVATE, NOFD) == 0);
    check("MAP_PHYS | MAP_ANON | MAP_SHARED: mapped", m(rw, MAP_PHYS | MAP_ANON | MAP_SHARED, NOFD) == 0);
    char *p = mmap(NULL, 4096, rw, MAP_SHARED, fd, 0);
    check("msync(0)", s(p, 0) == 0);
    check("msync(MS_SYNC)", s(p, MS_SYNC) == 0);
    check("msync(MS_ASYNC)", s(p, MS_ASYNC) == 0);
    check("msync(MS_SYNC | MS_ASYNC): EINVAL", s(p, MS_SYNC | MS_ASYNC) == EINVAL);
    check("msync(MS_CACHE_ONLY)", s(p, MS_CACHE_ONLY) == 0);
    check("msync(MS_SYNC | MS_CACHE_ONLY)", s(p, MS_SYNC | MS_CACHE_ONLY) == 0);
    check("msync(MS_ASYNC | MS_CACHE_ONLY)", s(p, MS_ASYNC | MS_CACHE_ONLY) == 0);
    check("msync(MS_SYNC | MS_INVALIDATE)", s(p, MS_SYNC | MS_INVALIDATE) == 0);
    check("msync(MS_SYNC_FULL)", s(p, MS_SYNC_FULL) == 0);
    check("msync(MS_SYNC | MS_SYNC_FULL): EINVAL", s(p, MS_SYNC | MS_SYNC_FULL) == EINVAL);
    check("msync(an unknown flag): EINVAL", s(p, 0x40000000) == EINVAL);
    printf("mmapflagtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

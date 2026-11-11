/* When a shared-memory object takes memory (review item PATHMEM-23), as
 * procnto (measured): when it is sized. ftruncate() of 64 MiB takes
 * 64 MiB of free memory at once (stat("/proc")'s size), and mapping and
 * touching it take nothing more; shm_ctl(SHMCTL_ANON) the same, with
 * SHMCTL_LAZY too (not typed memory); a size beyond free memory is ENOMEM
 * at ftruncate(), the object left empty. The kernel took the memory when
 * the object was first mapped or written, and let ftruncate() grow an
 * object past free memory. Free memory moves a little for page tables:
 * the checks allow 4 MiB. Prints one "ok"/"FAIL" line per check and
 * "shmbacktest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#define MIB (1024 * 1024L)

static int fails;
static long last;

static void check(const char *what, int ok)
{
    printf("shmbacktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static long free_mib(void)
{
    struct stat st;
    stat("/proc", &st);
    return (long)(st.st_size / MIB);
}

/* MiB taken since the last call */
static long taken(void)
{
    long f = free_mib(), t = last - f;
    last = f;
    return t;
}

int main(void)
{
    last = free_mib();
    int fd = shm_open("/shmbacktest", O_RDWR | O_CREAT, 0600);
    shm_unlink("/shmbacktest");
    taken();
    check("ftruncate(64 MiB)", ftruncate(fd, 64 * MIB) == 0);
    long t = taken();
    printf("shmbacktest: ftruncate took %ld MiB\n", t >= 60 && t <= 68 ? 64L : t);
    check("it takes the 64 MiB at once", t >= 60 && t <= 68);
    char *p = mmap(NULL, 64 * MIB, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    check("mmap", p != MAP_FAILED);
    if (p != MAP_FAILED) {
        for (long i = 0; i < 64 * MIB; i += 4096)
            p[i] = 1;
        check("mapping and touching it take nothing more", taken() <= 4);
        munmap(p, 64 * MIB);
    }
    close(fd);
    taken();

    fd = shm_open("/shmbacktest", O_RDWR | O_CREAT, 0600);
    shm_unlink("/shmbacktest");
    taken();
    check("shm_ctl(SHMCTL_ANON | SHMCTL_LAZY, 64 MiB)", shm_ctl(fd, SHMCTL_ANON | SHMCTL_LAZY, 0, 64 * MIB) == 0);
    t = taken();
    check("lazy or not, it takes the 64 MiB at once", t >= 60 && t <= 68);
    close(fd);
    taken();

    long big = free_mib() + 64;
    fd = shm_open("/shmbacktest", O_RDWR | O_CREAT, 0600);
    shm_unlink("/shmbacktest");
    taken();
    errno = 0;
    check("ftruncate() beyond free memory: ENOMEM", ftruncate(fd, big * MIB) == -1 && errno == ENOMEM);
    check("and nothing is taken", taken() <= 4);
    struct stat st;
    fstat(fd, &st);
    check("the object is left empty", st.st_size == 0);
    close(fd);
    printf("shmbacktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

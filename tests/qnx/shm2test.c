/* shm_ctl() on shared memory objects, as procnto's shmem_devctl:
 * - SHMCTL_ANON gives an object memory (the size asked, never less), which
 *   can then grow with the same flags but not be truncated (EINVAL);
 * - other flags on an object that has memory: EBADF; unknown flags,
 *   SHMCTL_LAZY with SHMCTL_PHYS: EINVAL;
 * - SHMCTL_ANON | SHMCTL_PHYS: physically contiguous memory
 *   (posix_mem_offset() covers it all);
 * - SHMCTL_PHYS: an object over given physical memory, which shows what
 *   the other mapping wrote;
 * - SHMCTL_ANON | SHMCTL_TYMEM: memory from a typed memory pool opened to
 *   allocate (contiguous with SHMCTL_PHYS, and then only from a pool
 *   opened with POSIX_TYPED_MEM_ALLOCATE_CONTIG), always the same pool;
 *   EBADF otherwise. SHMCTL_TYMEM isn't kept in the flags;
 * - SHMCTL_REPEAT: the physical run at the start, repeated a stride
 *   apart where the object has no memory yet; the object ends after the
 *   last copy (it may shrink);
 * - posix_mem_offset() where nothing is mapped: EACCES.
 * Prints one "ok"/"FAIL" line per check and "shm2test: passed". */
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
    printf("shm2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static off_t size_of(int fd)
{
    struct stat st;
    fstat(fd, &st);
    return st.st_size;
}

int main(void)
{
    int fd = shm_open("/shm2test_a", O_RDWR | O_CREAT, 0666);
    check("SHMCTL_ANON", shm_ctl(fd, SHMCTL_ANON, 0, 8192) == 0 && size_of(fd) == 8192);
    check("again, larger: it grows", shm_ctl(fd, SHMCTL_ANON, 0, 16384) == 0 && size_of(fd) == 16384);
    check("again, smaller: it stays", shm_ctl(fd, SHMCTL_ANON, 0, 4096) == 0 && size_of(fd) == 16384);
    errno = 0;
    check("ftruncate() of it: EINVAL", ftruncate(fd, 12288) == -1 && errno == EINVAL);
    errno = 0;
    check("other flags then: EBADF", shm_ctl(fd, SHMCTL_ANON | SHMCTL_LAZY, 0, 4096) == -1 && errno == EBADF);
    errno = 0;
    check("shm_ctl_special() then: EBADF", shm_ctl_special(fd, SHMCTL_ANON, 0, 4096, 0) == -1 && errno == EBADF);
    int f2 = shm_open("/shm2test_b", O_RDWR | O_CREAT, 0666);
    errno = 0;
    check("unknown flags: EINVAL", shm_ctl(f2, 0x12345678, 0, 4096) == -1 && errno == EINVAL);
    errno = 0;
    check("SHMCTL_LAZY | SHMCTL_PHYS: EINVAL",
          shm_ctl(f2, SHMCTL_ANON | SHMCTL_LAZY | SHMCTL_PHYS, 0, 4096) == -1 && errno == EINVAL);
    check("SHMCTL_ANON | SHMCTL_LAZY", shm_ctl(f2, SHMCTL_ANON | SHMCTL_LAZY, 0, 4096) == 0);

    /* Contiguous memory, and a second object over it. */
    int f3 = shm_open("/shm2test_c", O_RDWR | O_CREAT, 0666);
    check("SHMCTL_ANON | SHMCTL_PHYS", shm_ctl(f3, SHMCTL_ANON | SHMCTL_PHYS, 0, 4 * 4096) == 0);
    char *p = mmap(NULL, 4 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f3, 0);
    off64_t off = 0;
    size_t len = 0;
    int ofd;
    int r = posix_mem_offset64(p, 4 * 4096, &off, &len, &ofd);
    check("it is physically contiguous", r == 0 && len == 4 * 4096);
    strcpy(p + 4096, "through the other object");
    int f4 = shm_open("/shm2test_d", O_RDWR | O_CREAT, 0666);
    check("SHMCTL_PHYS over it", shm_ctl(f4, SHMCTL_PHYS, off + 4096, 4096) == 0 && size_of(f4) == 4096);
    char *q = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, f4, 0);
    check("the same memory", q != MAP_FAILED && !strcmp(q, "through the other object"));
    errno = 0;
    check("SHMCTL_PHYS unaligned: EINVAL", shm_ctl(f4, SHMCTL_PHYS, off + 1, 4096) == -1);

    /* Typed memory. */
    int tc = posix_typed_mem_open("/sysram", O_RDWR, POSIX_TYPED_MEM_ALLOCATE_CONTIG);
    int ta = posix_typed_mem_open("/sysram", O_RDWR, POSIX_TYPED_MEM_ALLOCATE);
    int tn = posix_typed_mem_open("/sysram", O_RDWR, POSIX_TYPED_MEM_MAP_ALLOCATABLE);
    int tb = posix_typed_mem_open("/below4G", O_RDWR, POSIX_TYPED_MEM_ALLOCATE);
    int f5 = shm_open("/shm2test_e", O_RDWR | O_CREAT, 0666);
    check("SHMCTL_TYMEM | SHMCTL_PHYS from an ALLOCATE_CONTIG pool",
          shm_ctl(f5, SHMCTL_ANON | SHMCTL_TYMEM | SHMCTL_PHYS, tc, 16384) == 0 && size_of(f5) == 16384 &&
              shm_ctl(f5, SHMCTL_GET_FLAGS, 0, 0) == (SHMCTL_ANON | SHMCTL_PHYS));
    char *t = mmap(NULL, 16384, PROT_READ | PROT_WRITE, MAP_SHARED, f5, 0);
    r = posix_mem_offset64(t, 16384, &off, &len, &ofd);
    check("it is contiguous and zeroed", r == 0 && len == 16384 && t[100] == 0);
    int f6 = shm_open("/shm2test_f", O_RDWR | O_CREAT, 0666);
    errno = 0;
    check("an ALLOCATE_CONTIG pool without SHMCTL_PHYS: EBADF",
          shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, tc, 16384) == -1 && errno == EBADF);
    errno = 0;
    check("an ALLOCATE pool with SHMCTL_PHYS: EBADF",
          shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM | SHMCTL_PHYS, ta, 16384) == -1 && errno == EBADF);
    errno = 0;
    check("a pool not opened to allocate: EBADF",
          shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, tn, 16384) == -1 && errno == EBADF);
    errno = 0;
    check("not a typed memory descriptor: EBADF", shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, f5, 16384) == -1 && errno == EBADF);
    check("SHMCTL_TYMEM from an ALLOCATE pool",
          shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, ta, 16384) == 0 && size_of(f6) == 16384 &&
              shm_ctl(f6, SHMCTL_GET_FLAGS, 0, 0) == SHMCTL_ANON);
    check("more from the same pool", shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, ta, 32768) == 0 && size_of(f6) == 32768);
    errno = 0;
    check("more from another pool: EBADF", shm_ctl(f6, SHMCTL_ANON | SHMCTL_TYMEM, tb, 65536) == -1 && errno == EBADF);

    /* A strided object over the contiguous memory: pages 0, 2 and 4 of it. */
    for (int i = 0; i < 4; i++) {
        p[i * 4096] = 'a' + i;
    }
    off64_t base;
    posix_mem_offset64(p, 4096, &base, &len, &ofd);
    int f7 = shm_open("/shm2test_g", O_RDWR | O_CREAT, 0666);
    errno = 0;
    check("SHMCTL_REPEAT of nothing: EINVAL", shm_ctl(f7, SHMCTL_REPEAT, 8192, 1) == -1 && errno == EINVAL);
    shm_ctl(f7, SHMCTL_PHYS, base, 4096);
    errno = 0;
    check("SHMCTL_REPEAT unaligned, or no count: EINVAL",
          shm_ctl(f7, SHMCTL_REPEAT, 8193, 1) == -1 && errno == EINVAL &&
              shm_ctl(f7, SHMCTL_REPEAT, 8192, 0) == -1 && errno == EINVAL);
    check("SHMCTL_REPEAT", shm_ctl(f7, SHMCTL_REPEAT, 8192, 1) == 0 && size_of(f7) == 8192);
    char *g = mmap(NULL, 8192, PROT_READ, MAP_SHARED, f7, 0);
    check("the pages a stride apart", g != MAP_FAILED && g[0] == 'a' && g[4096] == 'c');
    check("SHMCTL_REPEAT again: from the start", shm_ctl(f7, SHMCTL_REPEAT, 12288, 1) == 0 && size_of(f7) == 8192);
    char *g2 = mmap(NULL, 8192, PROT_READ, MAP_SHARED, f7, 0);
    check("the page that was there stays", g2 != MAP_FAILED && g2[4096] == 'c');
    int f8 = shm_open("/shm2test_h", O_RDWR | O_CREAT, 0666);
    shm_ctl(f8, SHMCTL_PHYS, base, 8192);
    errno = 0;
    check("a stride shorter than the run: EINVAL", shm_ctl(f8, SHMCTL_REPEAT, 4096, 1) == -1 && errno == EINVAL);

    check("posix_mem_offset() of nothing mapped: EACCES",
          posix_mem_offset64((void *)0x1000, 4096, &off, &len, &ofd) == EACCES);
    shm_unlink("/shm2test_a");
    shm_unlink("/shm2test_b");
    shm_unlink("/shm2test_c");
    shm_unlink("/shm2test_d");
    shm_unlink("/shm2test_e");
    shm_unlink("/shm2test_f");
    shm_unlink("/shm2test_g");
    shm_unlink("/shm2test_h");
    printf("shm2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

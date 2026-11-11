/* Memory calls with lengths and offsets near the top of the range (review
 * PATHMEM-6): the kernel used to add them up unchecked. The expectations
 * are procnto's measured answers: ENOMEM for anonymous, shared and
 * file mappings whose rounded size has no room, EINVAL for physical
 * ones and for an offset plus length that overflows, EINVAL from
 * munmap(), ENOMEM from mprotect(), msync() and mlock() (the range isn't
 * mapped). Prints one "ok"/"FAIL" line per call and "biglentest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/syspage.h>
#include <unistd.h>

static int fails;

/* `expr` fails with errno `err`. */
#define E(expr, err)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == -1 && errno == (err);                                                          \
        printf("biglentest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);           \
        fails += !ok_;                                                                                 \
    } while (0)
/* `expr` gives `val`. */
#define V(expr, val)                                                                                   \
    do {                                                                                               \
        errno = 0;                                                                                     \
        long r_ = (long)(expr);                                                                        \
        int ok_ = r_ == (long)(val);                                                                   \
        printf("biglentest: %s %s (%ld, errno %d)\n", ok_ ? "ok" : "FAIL", #expr, r_, errno);           \
        fails += !ok_;                                                                                 \
    } while (0)
#define M(...) (mmap(__VA_ARGS__) == MAP_FAILED ? -1 : 0)

int main(void)
{
    const uint64_t top = UINT64_MAX;
    E(M(NULL, top, PROT_READ, MAP_PRIVATE | MAP_ANON, -1, 0), ENOMEM);
    E(M(NULL, top - 4095, PROT_READ, MAP_PRIVATE | MAP_ANON, -1, 0), ENOMEM);
    E(M(NULL, top - 4095, PROT_READ | PROT_NOCACHE, MAP_PRIVATE | MAP_ANON, -1, 0), ENOMEM);
    E(M(NULL, top - 4095, PROT_READ, MAP_SHARED | MAP_ANON, -1, 0), ENOMEM);
    E(M((void *)0x10000, top - 0x10000, PROT_READ, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0), ENOMEM);
    E(M((void *)0x10001, top - 0x10000, PROT_READ, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0), ENOMEM);
    E(M(NULL, top - 4095, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, 0x1000), EINVAL);
    E(M(NULL, top - 4095, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, 0x1001), EINVAL);
    E(M(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, (off_t)(top - 4095)), EINVAL);
    /* MAP_FIXED past the end of the address space: ENOMEM */
    E(M((void *)(1ULL << 48), 4096, PROT_READ, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0), ENOMEM);
    E(M((void *)0x10000, 1ULL << 60, PROT_READ, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0), ENOMEM);
    E(M((void *)(1ULL << 48), 4096, PROT_READ, MAP_PRIVATE | MAP_PHYS | MAP_FIXED, -1, 0), ENOMEM);
    /* physical memory past the system page's root "memory" range: EINVAL */
    struct asinfo_entry *as = SYSPAGE_ENTRY(asinfo);
    uint64_t phys_end = 0;
    for (unsigned i = 0; i < SYSPAGE_ENTRY_SIZE(asinfo) / sizeof(*as); i++) {
        if (as[i].owner == AS_NULL_OFF) {
            phys_end = as[i].end;
            break;
        }
    }
    printf("biglentest: physical memory ends at %llx\n", (unsigned long long)phys_end);
    E(M(NULL, 8192, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, (off_t)(phys_end - 4095)), EINVAL);
    E(M(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, (off_t)(phys_end - 4094)), EINVAL);
    E(M(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, (off_t)(phys_end + 1)), EINVAL);
    void *last = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, (off_t)(phys_end - 4095));
    printf("biglentest: %s mmap of the last physical page (errno %d)\n", last == MAP_FAILED ? "FAIL" : "ok", errno);
    fails += last == MAP_FAILED;
    if (last != MAP_FAILED) {
        munmap(last, 4096);
    }
    E(munmap((void *)4096, top - 4096), EINVAL);
    E(munmap((void *)4097, top - 4096), EINVAL);
    E(munmap((void *)0x7fff00000000ULL, top - 0x7fff00000000ULL), EINVAL);
    E(mprotect((void *)4096, top - 4096, PROT_READ), ENOMEM);
    E(mprotect((void *)4097, top - 4096, PROT_READ), ENOMEM);
    E(msync((void *)4097, top - 4096, MS_SYNC), ENOMEM);
    E(mlock((void *)4097, top - 4096), ENOMEM);
    int fd = shm_open("/biglentest", O_RDWR | O_CREAT, 0600);
    E(shm_ctl(fd, SHMCTL_ANON | SHMCTL_PHYS, 0, top), ENOMEM);
    E(shm_ctl(fd, SHMCTL_ANON, 0, top - 4095), ENOMEM);
    E(shm_ctl(fd, SHMCTL_ANON, 4096, top - 4095), ENOMEM);
    E(shm_ctl(fd, SHMCTL_PHYS, 0x1001, 8192), EINVAL);
    E(ftruncate(fd, (off_t)top), EINVAL);
    E(ftruncate(fd, (off_t)INT64_MAX), ENOMEM);
    ftruncate(fd, 4096);
    E(M(NULL, top - 4095, PROT_READ, MAP_SHARED, fd, 0), ENOMEM);
    E(M(NULL, 4096, PROT_READ, MAP_SHARED, fd, (off_t)(top - 4095)), EINVAL);
    E(M(NULL, top - 4096, PROT_READ, MAP_SHARED, fd, 4097), EINVAL);
    close(fd);
    shm_unlink("/biglentest");
    /* a size of 0: accepted by shm_ctl(SHMCTL_PHYS), EINVAL from mmap() */
    fd = shm_open("/biglentest0", O_RDWR | O_CREAT, 0600);
    V(shm_ctl(fd, SHMCTL_PHYS, 0, 0), 0);
    V(shm_ctl(fd, SHMCTL_PHYS, 0x1000, 0), 0);
    close(fd);
    shm_unlink("/biglentest0");
    E(M(NULL, 0, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, 0), EINVAL);
    E(M(NULL, 0, PROT_READ, MAP_PRIVATE | MAP_PHYS, -1, 0x1000), EINVAL);
    E(M(NULL, 0, PROT_READ, MAP_PRIVATE | MAP_PHYS | MAP_ANON, -1, 0), EINVAL);
    fd = open("/proc/boot/biglentest", O_RDONLY);
    E(M(NULL, top - 4096, PROT_READ, MAP_PRIVATE, fd, 4097), EINVAL);
    E(M(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, (off_t)(top - 4095)), EINVAL);
    close(fd);
    printf("biglentest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

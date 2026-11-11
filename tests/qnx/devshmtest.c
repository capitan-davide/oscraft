/* A shared memory object over a device's registers (SHMCTL_PHYS), as
 * procnto answers (measured; review CORE-23), on QEMU's virt machine:
 * - over the PL031 real-time clock (0x9010000): read() reads the registers
 *   themselves: the data register, the time in seconds, moves between two
 *   reads more than a second apart; mmap() of the object reads them too;
 * - over an address no device answers at (0x9100000, between the
 *   machine's devices): a write() fails with ESRVRFAULT, and the system
 *   goes on.
 * QEMU only: a board's bus may answer an access to a missing device
 * differently (or later, asynchronously).
 * Prints one "ok"/"FAIL" line per check and "devshmtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("devshmtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    shm_unlink("/devshmtest");
    int fd = shm_open("/devshmtest", O_RDWR | O_CREAT, 0600);
    check("shm_ctl(SHMCTL_PHYS) at the real-time clock", fd >= 0 && shm_ctl(fd, SHMCTL_PHYS, 0x9010000, 4096) == 0);

    uint32_t a = 0, b = 0;
    int r1 = pread(fd, &a, 4, 0);
    usleep(1200000);
    int r2 = pread(fd, &b, 4, 0);
    check("read() reads the registers: the clock moves", r1 == 4 && r2 == 4 && b != a);

    volatile uint32_t *m = mmap(0, 4096, PROT_READ, MAP_SHARED, fd, 0);
    uint32_t c0 = m != MAP_FAILED ? m[0] : 0;
    usleep(1200000);
    check("mmap() reads the registers too", m != MAP_FAILED && m[0] != c0);
    close(fd);
    shm_unlink("/devshmtest");

    shm_unlink("/devshmtest-none");
    fd = shm_open("/devshmtest-none", O_RDWR | O_CREAT, 0600);
    check("shm_ctl(SHMCTL_PHYS) where no device is", fd >= 0 && shm_ctl(fd, SHMCTL_PHYS, 0x9100000, 4096) == 0);
    uint32_t z = 0;
    errno = 0;
    long w = pwrite(fd, &z, 4, 0x20);
    char what[100];
    snprintf(what, sizeof what, "a write no device takes: %ld errno %d (ESRVRFAULT %d)", w, errno, ESRVRFAULT);
    check(what, w == -1 && errno == ESRVRFAULT);
    close(fd);
    shm_unlink("/devshmtest-none");

    printf("devshmtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

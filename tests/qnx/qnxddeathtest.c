/*
 * What a QNX program sees of a native server when qnxd, the translation
 * service, is not there: never the server itself, whose FreeBSD replies
 * it could not read. On a descriptor it already has, ENOSYS; open()
 * takes that as "try the next server" and ends with ENOENT. Started
 * again, qnxd serves as before; the tests after this one need it.
 *
 * Not a procnto test: there is no qnxd on QNX. The expected values are
 * OsCraft's design (ker::msg, qnxd_for).
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("qnxddeathtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    struct stat st;
    int fd = open("/dev/random", O_RDONLY);
    check("open with qnxd there", fd >= 0);
    check("fstat with qnxd there", fstat(fd, &st) == 0);

    system("slay -f qnxd");
    /* qnxd's end is not synchronous with slay's. */
    int r = 0;
    for (int i = 0; i < 200 && (r = fstat(fd, &st)) == 0; i++) {
        struct timespec ts = { 0, 10 * 1000 * 1000 };
        nanosleep(&ts, NULL);
    }
    check("fstat with qnxd gone: ENOSYS", r == -1 && errno == ENOSYS);
    errno = 0;
    check("open with qnxd gone: ENOENT", open("/dev/random", O_RDONLY) == -1 && errno == ENOENT);

    system("/compat/qnx/sbin/qnxd");
    unsigned char b[16];
    int again = open("/dev/random", O_RDONLY);
    check("open with qnxd back", again >= 0);
    check("read with qnxd back", read(again, b, sizeof b) == (ssize_t)sizeof b);
    check("the old descriptor too", fstat(fd, &st) == 0);
    close(again);
    close(fd);
    printf("qnxddeathtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* A pipe as a QNX program sees it, procnto's results (measured): what is
 * waiting through FIONREAD and st_size, fpathconf()'s value in the reply
 * data, 5120 bytes written atomically, and poll() without POLLHUP. On
 * OsCraft these go through qnxd to the native pipe manager. */
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;
static char b[70000];

static void check(const char *what, int ok)
{
    printf("pipeinfotest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

int main(void)
{
    int p[2], n = -1;
    struct stat st;
    pipe(p);
    write(p[1], "hello", 5);
    check("FIONREAD", ioctl(p[0], FIONREAD, &n) == 0 && n == 5);
    check("st_size", fstat(p[0], &st) == 0 && st.st_size == 5);
    check("fpathconf(_PC_PIPE_BUF)", fpathconf(p[0], _PC_PIPE_BUF) == 5120);
    close(p[1]);
    struct pollfd pf = { p[0], POLLIN, 0 };
    check("poll: no POLLHUP", poll(&pf, 1, 0) == 1 && pf.revents == POLLRDNORM);
    close(p[0]);

    pipe(p);
    fcntl(p[1], F_SETFL, O_NONBLOCK);
    int total = 0;
    while ((n = write(p[1], b, 4096)) > 0) {
        total += n;
    }
    read(p[0], b, 3000);
    errno = 0;
    check("64 KB, and 4000 bytes are atomic", total == 65536 && write(p[1], b, 4000) == -1 && errno == EAGAIN);
    /* dup()s of ends of two pipes share a server connection ID: each still
     * is its own end (ssh's stdin and stdout, from scp). */
    int a[2], c[2];
    pipe(a);
    pipe(c);
    int rd = dup(a[0]), wr = dup(c[1]);
    fcntl(rd, F_SETFL, O_NONBLOCK);
    struct pollfd two[2] = { { rd, POLLIN, 0 }, { wr, POLLOUT, 0 } };
    check("poll() of dups of two pipes", poll(two, 2, 300) == 1 && !two[0].revents && (two[1].revents & POLLOUT));
    check("O_NONBLOCK stays on its own pipe", !(fcntl(wr, F_GETFL) & O_NONBLOCK));
    printf("pipeinfotest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

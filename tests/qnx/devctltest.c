/* _IO_DEVCTL on procnto's files (review item PATHMEM-17), as procnto
 * (measured), with raw messages: a devctl none of them knows is ENOSYS
 * (libc's devctl() and ioctl() make it ENOTTY); a shared memory file's
 * F_SETFL or shm_ctl() with less data than nbytes says is EBADMSG; a short
 * F_SETFL on /dev/null or /proc is done; shm_ctl()'s devctl on /dev/null
 * or /proc is ENOSYS. The kernel answered ENOTTY to all of them. Prints
 * one "ok"/"FAIL" line per check and "devctltest: passed". */
#include <devctl.h>
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/dcmd_all.h>
#include <sys/ioctl.h>
#include <sys/iomsg.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <unistd.h>

#define SHM_CTL 0x80180864u

static int fails;

static void check(const char *what, int ok)
{
    printf("devctltest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* a raw _IO_DEVCTL of `dcmd` saying `nbytes`, with `sent` bytes of
 * data: 0 or the errno */
static int raw(int fd, unsigned dcmd, unsigned nbytes, unsigned sent)
{
    struct {
        struct _io_devctl hdr;
        char data[64];
    } m;
    memset(&m, 0, sizeof m);
    m.hdr.type = _IO_DEVCTL;
    m.hdr.combine_len = sizeof m.hdr;
    m.hdr.dcmd = dcmd;
    m.hdr.nbytes = nbytes;
    char reply[128];
    return MsgSend(fd, &m, sizeof m.hdr + sent, reply, sizeof reply) == -1 ? errno : 0;
}

int main(void)
{
    char line[120];
    int s = shm_open("/devctltest", O_RDWR | O_CREAT, 0600);
    shm_unlink("/devctltest");
    int fds[] = { s, open("/dev/null", O_RDWR), open("/proc/self/as", O_RDONLY) };
    const char *names[] = { "a shm object", "/dev/null", "/proc/self/as" };
    for (int i = 0; i < 3; i++) {
        snprintf(line, sizeof line, "%s: an unknown devctl: ENOSYS", names[i]);
        check(line, raw(fds[i], 0x12345678, 0, 0) == ENOSYS);
        snprintf(line, sizeof line, "%s: libc's devctl() and ioctl() of it: ENOTTY", names[i]);
        errno = 0;
        check(line, devctl(fds[i], 0x12345678, NULL, 0, NULL) == ENOTTY && ioctl(fds[i], 0x12345678, 0) == -1 &&
                        errno == ENOTTY);
    }
    check("a shm object: F_SETFL with no data: EBADMSG", raw(s, DCMD_ALL_SETFLAGS, 4, 0) == EBADMSG);
    check("a shm object: shm_ctl() with 8 of 24 bytes: EBADMSG", raw(s, SHM_CTL, 24, 8) == EBADMSG);
    check("a shm object: F_SETFL with its data: done", raw(s, DCMD_ALL_SETFLAGS, 4, 4) == 0);
    for (int i = 1; i < 3; i++) {
        snprintf(line, sizeof line, "%s: F_SETFL with no data: done", names[i]);
        check(line, raw(fds[i], DCMD_ALL_SETFLAGS, 4, 0) == 0);
        snprintf(line, sizeof line, "%s: shm_ctl(): ENOSYS", names[i]);
        check(line, raw(fds[i], SHM_CTL, 24, 24) == ENOSYS);
    }

    printf("devctltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

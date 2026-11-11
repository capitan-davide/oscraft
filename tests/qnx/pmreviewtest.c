/* Process manager details of review #185, as procnto answers (measured):
 * - PROCMGR-17: _PROC_GETNAME whose name and NUL don't fit the reply
 *   buffer: only the NUL, the status still the name's length;
 * - PROCMGR-23: posix_spawn() with POSIX_SPAWN_SETPGROUP into a group
 *   that doesn't exist: EPERM; into the caller's group, or 0 (a new one):
 *   spawned, in it;
 * - PROCMGR-18: tcsetattr() keeps the speeds; a termios shorter than
 *   struct termios (DCMD_CHR_TCSETATTR of 40 bytes): EBADMSG;
 * - PROCMGR-22: tcsetpgrp() from a process in another session: ENOTTY,
 *   and the terminal's group stays.
 * Prints one "ok"/"FAIL" line per check and "pmreviewtest: passed". */
#include <devctl.h>
#include <errno.h>
#include <spawn.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/dcmd_chr.h>
#include <sys/neutrino.h>
#include <sys/procmsg.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

extern char **environ;
static int fails;
static char *self;

static void check(const char *what, int ok)
{
    printf("pmreviewtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void getname(void)
{
    unsigned char m[16] = {0x23, 0};
    uint64_t me = getpid64(getpid());
    memcpy(m + 8, &me, 8);
    unsigned char r[64];
    memset(r, 0xee, sizeof r);
    long st = MsgSend(SYSMGR_COID, m, sizeof m, r, 4);
    check("_PROC_GETNAME into 4 bytes: only the NUL, the status the name's length",
          st > 4 && r[0] == 0 && r[1] == 0xee);
    memset(r, 0xee, sizeof r);
    long full = MsgSend(SYSMGR_COID, m, sizeof m, r, sizeof r);
    check("and with room: the name and its NUL", full == st && r[full] == 0 && r[0] != 0);
}

/* 0: spawned in group `g`; 1: spawned elsewhere; else the errno. */
static int spawn_in(pid_t g)
{
    posix_spawnattr_t a;
    posix_spawnattr_init(&a);
    posix_spawnattr_setflags(&a, POSIX_SPAWN_SETPGROUP);
    posix_spawnattr_setpgroup(&a, g);
    char *argv[] = {self, "child", NULL};
    pid_t p;
    int e = posix_spawnp(&p, self, NULL, &a, argv, environ);
    if (e)
        return e;
    int st;
    waitpid(p, &st, 0);
    return WEXITSTATUS(st) == 7 ? 0 : 1;
}

static void terminal(void)
{
    int fd = 0;
    struct termios t, u, v;
    if (tcgetattr(fd, &t) != 0) {
        check("tcgetattr() on the terminal", 0);
        return;
    }
    u = t;
    cfsetispeed(&u, B9600);
    cfsetospeed(&u, B9600);
    int r = tcsetattr(fd, TCSANOW, &u);
    tcgetattr(fd, &v);
    check("tcsetattr() keeps the speeds", r == 0 && cfgetispeed(&v) == B9600 && cfgetospeed(&v) == B9600);
    tcsetattr(fd, TCSANOW, &t);
    check("a termios of 40 bytes: EBADMSG", devctl(fd, DCMD_CHR_TCSETATTR, &t, 40, NULL) == EBADMSG);
    pid_t k = fork();
    if (k == 0) {
        setsid();
        int r1 = tcsetpgrp(fd, getpgrp());
        _exit(r1 == -1 && errno == ENOTTY ? 0 : 1);
    }
    int st;
    waitpid(k, &st, 0);
    check("tcsetpgrp() from another session: ENOTTY", WIFEXITED(st) && WEXITSTATUS(st) == 0);
    check("and the terminal's group stays", tcgetpgrp(fd) == getpgrp());
}

int main(int argc, char **argv)
{
    if (argc > 1)
        return getpgrp() == getpid() || getpgrp() == getpgid(getppid()) ? 7 : 8;
    self = argv[0];
    getname();
    check("spawn into a group that doesn't exist: EPERM", spawn_in(99999) == EPERM);
    check("into the caller's group: in it", spawn_in(getpgrp()) == 0);
    check("into 0: a new group, in it", spawn_in(0) == 0);
    terminal();
    printf("pmreviewtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

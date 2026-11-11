/* Controlling terminals, as sshd uses them (pty_make_controlling_tty):
 * - after setsid() the session has no terminal: /dev/tty is ENXIO;
 * - TIOCSCTTY on a pty makes it the session's: /dev/tty opens, and
 *   tcgetsid() and tcgetpgrp() give the session;
 * - when the session leader ends, the pty is free for another session,
 *   though other processes keep it open (sshd does);
 * - and the processes of its foreground group get SIGHUP.
 * Prints one "ok"/"FAIL" line per check and "ttytest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>
#include <unix.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("ttytest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* A new session in a child that makes `s` its terminal; the child's
 * results, one byte each: /dev/tty before, TIOCSCTTY, /dev/tty after,
 * tcgetsid() and tcgetpgrp() right. */
static void session(int s, char out[5])
{
    int p[2];
    pipe(p);
    pid_t pid = fork();
    if (pid == 0) {
        char r[5];
        setsid();
        errno = 0;
        r[0] = open("/dev/tty", O_RDWR | O_NOCTTY) == -1 && errno == ENXIO;
        r[1] = ioctl(s, TIOCSCTTY, NULL) == 0;
        r[2] = open("/dev/tty", O_WRONLY) >= 0;
        r[3] = tcgetsid(s) == getpid();
        r[4] = tcgetpgrp(s) == getpid();
        write(p[1], r, sizeof r);
        _exit(0);
    }
    memset(out, 0, 5);
    read(p[0], out, 5);
    waitpid(pid, NULL, 0);
    close(p[0]);
    close(p[1]);
}

int main(void)
{
    int m, s;
    if (openpty(&m, &s, NULL, NULL, NULL) == -1) {
        printf("ttytest: openpty: %s\n", strerror(errno));
        return 1;
    }
    char r[5];
    session(s, r);
    check("a new session has no terminal: ENXIO", r[0]);
    check("TIOCSCTTY", r[1]);
    check("then /dev/tty opens", r[2]);
    check("tcgetsid(), tcgetpgrp(): the session", r[3] && r[4]);
    session(s, r);
    check("the pty again for the next session (still open here)", r[1] && r[2]);

    /* The foreground group hears of its leader's end. */
    int p[2];
    pipe(p);
    pid_t leader = fork();
    if (leader == 0) {
        setsid();
        ioctl(s, TIOCSCTTY, NULL);
        pid_t member = fork();
        if (member == 0) {
            sigset_t set;
            sigemptyset(&set);
            sigaddset(&set, SIGHUP);
            sigprocmask(SIG_BLOCK, &set, NULL);
            write(p[1], "r", 1);
            int sig = 0;
            struct timespec t = {2, 0};
            siginfo_t info;
            sig = sigtimedwait(&set, &info, &t);
            char c = sig == SIGHUP ? 'h' : 'n';
            write(p[1], &c, 1);
            _exit(0);
        }
        char c;
        read(p[0], &c, 1);
        _exit(0);
    }
    waitpid(leader, NULL, 0);
    char c = 0;
    read(p[0], &c, 1);
    check("the foreground group gets SIGHUP when the leader ends", c == 'h');
    printf("ttytest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

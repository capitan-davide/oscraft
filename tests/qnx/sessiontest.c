/* Controlling terminals (as procnto): a session leader makes a terminal
 * its session's with tcsetsid(); tcgetsid() then names the session and
 * /dev/tty opens that terminal. Uses a pseudo-terminal (devc-pty).
 * procmgr_session()'s signals: SIGNAL_PID signals the process group of a
 * process unless it ignores the signal, or thread xid blocks it (EIO;
 * EINVAL for a bad signal or no process), SIGNAL_LEADER needs a terminal
 * (EINVAL), SIGNAL_PGRP a session.
 * Prints one "ok"/"FAIL" line per check and "sessiontest: passed". */
#include <errno.h>
#include <signal.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/procmsg.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <termios.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("sessiontest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* In a new session: 0 if all went as expected, else the step that failed. */
static int leader(void)
{
    if (setsid() == -1) {
        return 1;
    }
    int master = open("/dev/ptyp0", O_RDWR);
    int slave = open("/dev/ttyp0", O_RDWR | O_NOCTTY);
    if (master < 0 || slave < 0) {
        return 2;
    }
    if (tcsetsid(slave, getpid()) != 0) {
        printf("sessiontest:   tcsetsid: %d\n", errno);
        return 3;
    }
    if (tcgetsid(slave) != getpid()) {
        return 4;
    }
    int tty = open("/dev/tty", O_RDWR);
    struct stat a, b;
    if (tty < 0 || fstat(tty, &a) != 0 || fstat(slave, &b) != 0) {
        return 5;
    }
    if (a.st_ino != b.st_ino || a.st_dev != b.st_dev) {
        return 6;
    }
    return 0;
}

/* Not a session leader: EPERM. */
static int not_leader(void)
{
    int master = open("/dev/ptyp1", O_RDWR);
    int slave = open("/dev/ttyp1", O_RDWR | O_NOCTTY);
    if (master < 0 || slave < 0) {
        return 2;
    }
    int r = tcsetsid(slave, getpid());
    return r == -1 && errno == EPERM ? 0 : 1;
}

static int in_child(int (*fn)(void))
{
    pid_t pid = fork();
    if (pid == 0) {
        _exit(fn());
    }
    int st;
    waitpid(pid, &st, 0);
    return WIFEXITED(st) ? WEXITSTATUS(st) : 255;
}

static int session_err(pid_t sid, int signo, unsigned event)
{
    return procmgr_session(0, sid, signo, event) == -1 ? errno : 0;
}

/* _PROC_SESSION with a thread ID in xid (procmgr_session() sends 0). */
static int session_xid(int xid, pid_t sid, int signo, unsigned event)
{
    proc_session_t m;
    memset(&m, 0, sizeof m);
    m.i.type = _PROC_SESSION;
    m.i.xid = xid;
    m.i.sid = sid;
    m.i.id = signo;
    m.i.event = event;
    return MsgSend(PROCMGR_COID, &m.i, sizeof m.i, NULL, 0) == -1 ? errno : 0;
}

static void signals(void)
{
    signal(SIGCONT, SIG_IGN);
    pid_t c = fork();
    if (c == 0) {
        setsid();
        signal(SIGCONT, SIG_IGN);
        for (;;) {
            pause();
        }
    }
    usleep(50000);
    check("SIGNAL_PID: a process not ignoring the signal",
          session_err(getsid(0), SIGCONT, PROCMGR_SESSION_SIGNAL_PID) == 0 &&
              session_err(c, SIGWINCH, PROCMGR_SESSION_SIGNAL_PID) == 0);
    check("SIGNAL_PID: a process ignoring the signal EIO",
          session_err(getpid(), SIGCONT, PROCMGR_SESSION_SIGNAL_PID) == EIO &&
              session_err(c, SIGCONT, PROCMGR_SESSION_SIGNAL_PID) == EIO);
    sigset_t block, old;
    sigemptyset(&block);
    sigaddset(&block, SIGWINCH);
    sigprocmask(SIG_BLOCK, &block, &old);
    check("SIGNAL_PID: thread xid blocking the signal EIO",
          session_xid(gettid(), getpid(), SIGWINCH, PROCMGR_SESSION_SIGNAL_PID) == EIO &&
              session_err(getpid(), SIGWINCH, PROCMGR_SESSION_SIGNAL_PID) == 0);
    sigprocmask(SIG_SETMASK, &old, NULL);
    check("SIGNAL_PID: signal 0 or no such process EINVAL",
          session_err(getpid(), 0, PROCMGR_SESSION_SIGNAL_PID) == EINVAL &&
              session_err(99999, SIGCONT, PROCMGR_SESSION_SIGNAL_PID) == EINVAL);
    check("SIGNAL_LEADER: a session without a terminal EINVAL",
          session_err(c, SIGCONT, PROCMGR_SESSION_SIGNAL_LEADER) == EINVAL);
    check("SIGNAL_PGRP: any session; not a session EINVAL",
          session_err(c, SIGCONT, PROCMGR_SESSION_SIGNAL_PGRP) == 0 &&
              session_err(getpid(), SIGCONT, PROCMGR_SESSION_SIGNAL_PGRP) == EINVAL);
    kill(c, SIGKILL);
    waitpid(c, NULL, 0);
}

int main(void)
{
    int r = in_child(leader);
    if (r != 0) {
        printf("sessiontest:   leader: step %d failed\n", r);
    }
    check("tcsetsid(): the session's terminal, tcgetsid() and /dev/tty agree", r == 0);
    check("tcsetsid() from a process that leads no session: EPERM", in_child(not_leader) == 0);
    signals();
    printf("sessiontest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

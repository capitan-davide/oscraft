/* Process manager events (procmgr_event_notify() & co.), as procnto
 * delivers them. Each kind has its own notification (and pulse code), so
 * the processes the image's scripts start don't get in the way:
 * - PROCESS_CREATE and PROCESS_DEATH with an updateable event carry the
 *   pid; a plain event keeps its own value;
 * - a spawn that fails (a missing program, a device) still uses up a pid:
 *   its PROCESS_DEATH comes without PROCESS_CREATE, and it leaves no
 *   child to wait for;
 * - PATHSPACE carries the documented hash of the attached path, on
 *   attach and on detach;
 * - TOD comes from clock_settime(), CONFSTR from sethostname() (its value
 *   is _CS_HOSTNAME);
 * - procmgr_event_trigger() gives value 0, _updateable() the value given,
 *   sync() triggers SYNC; root may trigger the privileged events, other
 *   users get EPERM;
 * - procmgr_event_notify_delete() of an unknown id: ESRCH; a deleted
 *   notification is quiet;
 * - an event not registered (MsgRegisterEvent()) is accepted but never
 *   delivered, even with procnto -U.
 * Prints one "ok"/"FAIL" line per check and "pevtest: passed". */
#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;
static int fails;
static int chid, coid;

static void check(const char *what, int ok)
{
    printf("pevtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* A registered pulse event with `code`, updateable or not. */
static struct sigevent event(int code, int value, int updateable)
{
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, coid, 10, code, value);
    if (updateable) {
        SIGEV_MAKE_UPDATEABLE(&ev);
    }
    MsgRegisterEvent(&ev, SYSMGR_COID);
    return ev;
}

/* The pulses received and not yet looked for. */
static struct _pulse got[64];
static int ngot;

/* Wait (up to 2 s) for a pulse with `code` and `value`; the others are
 * kept for later checks. */
static int seen(int code, int value)
{
    for (int i = 0; i < ngot; i++) {
        if (got[i].code == code && got[i].value.sival_int == value) {
            got[i] = got[--ngot];
            return 1;
        }
    }
    struct _pulse p;
    for (;;) {
        uint64_t t = 2000000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &t, NULL);
        if (MsgReceivePulse(chid, &p, sizeof p, NULL) == -1) {
            return 0;
        }
        if (p.code == code && p.value.sival_int == value) {
            return 1;
        }
        if (ngot < 64) {
            got[ngot++] = p;
        }
    }
}

/* Whether a PROCESS_DEATH (code 2) comes within 300 ms for a pid without
 * PROCESS_CREATE (code 1). */
static int death_alone(void)
{
    int created[64], nc = 0, died[64], nd = 0;
    struct _pulse p;
    for (;;) {
        if (ngot > 0) {
            p = got[--ngot];
        } else {
            uint64_t t = 300000000;
            TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &t, NULL);
            if (MsgReceivePulse(chid, &p, sizeof p, NULL) == -1) {
                break;
            }
        }
        if (p.code == 1 && nc < 64) {
            created[nc++] = p.value.sival_int;
        } else if (p.code == 2 && nd < 64) {
            died[nd++] = p.value.sival_int;
        }
    }
    for (int i = 0; i < nd; i++) {
        int c = 0;
        for (int j = 0; j < nc; j++) {
            c |= created[j] == died[i];
        }
        if (!c) {
            return 1;
        }
    }
    return 0;
}

/* Whether a pulse with `code` comes within 300 ms (those kept count). */
static int any(int code)
{
    for (int i = 0; i < ngot; i++) {
        if (got[i].code == code) {
            return 1;
        }
    }
    struct _pulse p;
    for (;;) {
        uint64_t t = 300000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &t, NULL);
        if (MsgReceivePulse(chid, &p, sizeof p, NULL) == -1) {
            return 0;
        }
        if (p.code == code) {
            return 1;
        }
    }
}

static unsigned pathspace_hash(const char *s)
{
    unsigned h = 0, x;
    for (; *s; ++s) {
        h = (h << 4) + (unsigned char)*s;
        if ((x = h & 0xf0000000u) != 0) {
            h ^= x >> 24;
            h &= ~x;
        }
    }
    return h | 0x80000000u;
}

int main(void)
{
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);

    struct sigevent create = event(1, 0, 1), death = event(2, 0, 1), plain = event(3, 0x33, 0);
    check("procmgr_event_notify()", procmgr_event_notify(PROCMGR_EVENT_PROCESS_CREATE, &create) == 0);
    int did = procmgr_event_notify_add(PROCMGR_EVENT_PROCESS_DEATH, &death);
    int pid_ = procmgr_event_notify_add(PROCMGR_EVENT_PROCESS_DEATH, &plain);
    check("procmgr_event_notify_add()", did >= 0 && pid_ >= 0 && did != pid_);
    pid_t pid;
    char *argv[] = {"sleep", "0", NULL};
    posix_spawnp(&pid, "sleep", NULL, NULL, argv, environ);
    check("PROCESS_CREATE carries the new pid", seen(1, pid));
    waitpid(pid, NULL, 0);
    check("PROCESS_DEATH carries the pid", seen(2, pid));
    check("a plain event keeps its value", seen(3, 0x33));
    char *nargv[] = {"pevtest-none", NULL};
    check("a missing program: ENOENT", posix_spawn(&pid, "/proc/boot/pevtest-none", NULL, NULL, nargv, environ) == ENOENT);
    check("a pid's PROCESS_DEATH, without PROCESS_CREATE", death_alone());
    check("no child to wait for", waitpid(-1, NULL, WNOHANG) == -1 && errno == ECHILD);
    char *dargv[] = {"null", NULL};
    check("a device: EACCES", posix_spawn(&pid, "/dev/null", NULL, NULL, dargv, environ) == EACCES);
    check("a pid's PROCESS_DEATH again", death_alone());
    procmgr_event_notify_delete(pid_);
    procmgr_event_notify_delete(did);
    procmgr_event_notify(0, &create);

    struct sigevent path = event(4, 0, 1);
    procmgr_event_notify(PROCMGR_EVENT_PATHSPACE, &path);
    unsigned hash = pathspace_hash("/dev/name/local/pevtest_name");
    name_attach_t *na = name_attach(NULL, "pevtest_name", 0);
    check("PATHSPACE on attach: the path's hash", seen(4, hash));
    name_detach(na, 0);
    check("PATHSPACE on detach: the path's hash", seen(4, hash));

    struct sigevent tod = event(5, 0x55, 1);
    procmgr_event_notify(PROCMGR_EVENT_TOD, &tod);
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    clock_settime(CLOCK_REALTIME, &ts);
    check("TOD from clock_settime(), value 0", seen(5, 0));

    struct sigevent conf = event(6, 0, 1);
    procmgr_event_notify(PROCMGR_EVENT_CONFSTR, &conf);
    char host[256] = "";
    gethostname(host, sizeof host);
    sethostname("pevtest", 7);
    check("CONFSTR from sethostname(): _CS_HOSTNAME", seen(6, _CS_HOSTNAME));
    sethostname(host, strlen(host));

    struct sigevent sync_ev = event(7, 0x77, 1);
    procmgr_event_notify(PROCMGR_EVENT_SYNC | PROCMGR_EVENT_SYSCONF, &sync_ev);
    check("procmgr_event_trigger(): value 0",
          procmgr_event_trigger(PROCMGR_EVENT_SYNC) == 0 && seen(7, 0));
    check("procmgr_event_trigger_updateable(): its value",
          procmgr_event_trigger_updateable(PROCMGR_EVENT_SYNC, 1234) == 0 && seen(7, 1234));
    sync();
    check("sync(): SYNC", seen(7, 0));
    check("root triggers SYSCONF", procmgr_event_trigger(PROCMGR_EVENT_SYSCONF) == 0 && seen(7, 0));

    int fds[2];
    pipe(fds);
    pid = fork();
    if (pid == 0) {
        setuid(100);
        errno = 0;
        int r = procmgr_event_trigger(PROCMGR_EVENT_SYSCONF);
        char c = r == -1 && errno == EPERM;
        write(fds[1], &c, 1);
        _exit(0);
    }
    char c = 0;
    read(fds[0], &c, 1);
    waitpid(pid, NULL, 0);
    check("another user triggers SYSCONF: EPERM", c == 1);

    errno = 0;
    check("procmgr_event_notify_delete() of no such id: ESRCH",
          procmgr_event_notify_delete(12345) == -1 && errno == ESRCH);
    procmgr_event_notify(0, &sync_ev);
    procmgr_event_trigger(PROCMGR_EVENT_SYNC);
    check("a removed notification is quiet", !any(7));

    struct sigevent unreg;
    SIGEV_PULSE_INIT(&unreg, coid, 10, 9, 0);
    check("an unregistered event: accepted", procmgr_event_notify(PROCMGR_EVENT_SYNC, &unreg) == 0);
    procmgr_event_trigger(PROCMGR_EVENT_SYNC);
    check("but never delivered", !any(9));
    procmgr_event_notify(0, &unreg);

    printf("pevtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

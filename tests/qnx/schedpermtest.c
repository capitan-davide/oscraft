/* SchedGet() and SchedSet() on another process's thread across user IDs
 * (review item SCHED-4), as procnto's sched_thread_lookup() decides it
 * (nto_cred_is_allowed): allowed when the caller's effective user ID
 * equals the target's real, effective and saved ones, all three; else
 * EPERM for both calls, unless the caller has the PROCMGR_AID_SCHEDULE
 * ability in its non-root domain. Run as root: children at uid 100
 * (all three IDs), at ruid 0 / euid 100 (seteuid), and at uid 100
 * against a parent at ruid 100 / euid 0 (setreuid) try the parent's
 * thread 1. procnto's own threads (review item SCHED-32): SchedGet()
 * works, SchedSet() is EPERM even for root, and for a thread procnto
 * doesn't have, so before the lookup (procnto, measured; the kernel used
 * to answer EINVAL, or ESRCH). Prints one "ok"/"FAIL" line per check and
 * "schedpermtest: passed". */
#include <errno.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("schedpermtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Returns the child's exit status: its number of failed checks. */
/* give_ability: PROCMGR_AID_SCHEDULE in the non-root domain first;
 * effective_only: seteuid(100) (ruid stays 0) instead of setuid(100). */
static int in_child(const char *who, int give_ability, int effective_only)
{
    pid_t pid = fork();
    if (pid == 0) {
        char what[96];
        int f0 = fails;
        if (give_ability) {
            snprintf(what, sizeof what, "%s: SCHEDULE allowed for non-root", who);
            check(what, procmgr_ability(0, PROCMGR_ADN_NONROOT | PROCMGR_AOP_ALLOW | PROCMGR_AID_SCHEDULE,
                                        PROCMGR_AID_EOL) == 0);
        }
        if (effective_only) {
            snprintf(what, sizeof what, "%s: seteuid(100), ruid 0", who);
            check(what, seteuid(100) == 0 && geteuid() == 100 && getuid() == 0);
        } else {
            snprintf(what, sizeof what, "%s: setuid(100)", who);
            check(what, setuid(100) == 0 && geteuid() == 100);
        }
        struct sched_param p = { 0 };
        errno = 0;
        int policy = SchedGet(getppid(), 1, &p);
        struct sched_param np = { 0 };
        np.sched_priority = 9;
        errno = 0;
        int r = SchedSet(getppid(), 1, SCHED_NOCHANGE, &np);
        int err = r ? errno : 0;
        if (give_ability) {
            snprintf(what, sizeof what, "%s: SchedGet of root's thread allowed", who);
            check(what, policy >= 0 && p.sched_priority == 10);
            snprintf(what, sizeof what, "%s: SchedSet of root's thread allowed", who);
            check(what, r == 0);
            np.sched_priority = 10;
            SchedSet(getppid(), 1, SCHED_NOCHANGE, &np);
        } else {
            snprintf(what, sizeof what, "%s: SchedGet of root's thread: EPERM", who);
            check(what, policy == -1 && errno == EPERM);
            snprintf(what, sizeof what, "%s: SchedSet of root's thread: EPERM", who);
            check(what, r == -1 && err == EPERM);
            /* a bad policy is still EPERM: the permission comes first */
            errno = 0;
            r = SchedSet(getppid(), 1, 99, &np);
            snprintf(what, sizeof what, "%s: with a bad policy too: EPERM, not EINVAL", who);
            check(what, r == -1 && errno == EPERM);
        }
        errno = 0;
        r = SchedSet(0, 0, SCHED_NOCHANGE, &np);
        snprintf(what, sizeof what, "%s: its own thread: allowed", who);
        check(what, r == 0);
        _exit(fails - f0);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 99;
}

int main(void)
{
    struct sched_param p;
    check("running as root", geteuid() == 0);
    /* a root child: same user */
    pid_t pid = fork();
    if (pid == 0) {
        struct sched_param np = { 0 };
        np.sched_priority = 9;
        int r = SchedSet(getppid(), 1, SCHED_NOCHANGE, &np);
        SchedGet(getppid(), 1, &np);
        int ok = r == 0 && np.sched_priority == 9;
        np.sched_priority = 10;
        SchedSet(getppid(), 1, SCHED_NOCHANGE, &np);
        _exit(!ok);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    check("same user (root child): SchedSet and SchedGet of the parent's thread", WEXITSTATUS(status) == 0);
    fails += in_child("uid 100", 0, 0);
    fails += in_child("uid 100 with the ability", 1, 0);
    /* the caller's effective ID is what counts: ruid 0 doesn't help */
    fails += in_child("ruid 0, euid 100", 0, 1);
    /* and the target's three IDs must all match it: a parent at
     * ruid 100 / euid 0 is not uid 100's to schedule */
    check("parent: setreuid(100, 0)", setreuid(100, 0) == 0 && getuid() == 100 && geteuid() == 0);
    fails += in_child("uid 100 vs parent ruid 100 euid 0", 0, 0);
    check("parent: back to root", setreuid(0, 0) == 0 && getuid() == 0);
    SchedGet(0, 1, &p);
    check("the parent's priority is back to 10", p.sched_priority == 10);
    /* procnto's own threads: readable, never set, not even by root, and
     * refused before the thread is looked up (procnto, measured) */
    struct sched_param pp;
    check("SchedGet() of procnto's thread 1", SchedGet(1, 1, &pp) != -1);
    errno = 0;
    check("SchedSet() of procnto's thread 1: EPERM", SchedSet(1, 1, SCHED_NOCHANGE, &pp) == -1 && errno == EPERM);
    errno = 0;
    check("SchedSet() of a thread procnto doesn't have: EPERM",
          SchedSet(1, 9999, SCHED_NOCHANGE, &pp) == -1 && errno == EPERM);
    errno = 0;
    check("SchedGet() of a thread procnto doesn't have: ESRCH", SchedGet(1, 9999, &pp) == -1 && errno == ESRCH);
    SchedGet(0, 0, &p);
    errno = 0;
    check("SchedSet(1, 0): thread 0 is the caller's own", SchedSet(1, 0, SCHED_NOCHANGE, &p) == 0);
    printf("schedpermtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

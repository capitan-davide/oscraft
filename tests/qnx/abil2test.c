/* Custom abilities and servers' checks of their clients' abilities, as
 * procnto (measured in the reference image):
 * - procnto makes iofunc/chown, iofunc/read, iofunc/dup (root's) and
 *   iofunc/exec (nobody's) first, after QNX's own abilities (76-79);
 * - procmgr_ability_create() flags name the domains that have the ability
 *   by default; anything else is EINVAL; creating it again with flags
 *   lacking one of its domains is EEXIST, else the same ID;
 * - without PROCMGR_AID_ABLE_CREATE a new name can't be created (EPERM)
 *   and a lookup gets libc's temporary ID (PROCMGR_AID_UNCREATED), which
 *   procmgr_ability() refuses (EINVAL);
 * - procmgr_ability_name() names custom abilities only (EINVAL), and
 *   fails if the buffer is too small;
 * - ConnectClientInfoAble() sets each entry's flags to 1 if the client
 *   has the ability for the whole range, 0 if not (unknown IDs too), and
 *   _NTO_CI_UNABLE if any is 0; no entries is EFAULT; a chroot()ed client
 *   has _NTO_CI_CHROOT.
 * Prints one "ok"/"FAIL" line per check and "abil2test: passed". */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("abil2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int chid;
static unsigned ci_flags;
static int ci_err;

/* Fork a client that runs `setup` and sends one message; check `n`
 * abilities of it with ConnectClientInfoAble(). The flags of the entries
 * as a bit string ('1'/'0'). */
static const char *ask(void (*setup)(void), struct _client_able *a, int n)
{
    static char bits[16];
    pid_t pid = fork();
    if (pid == 0) {
        if (setup) {
            setup();
        }
        int coid = ConnectAttach(0, getppid(), chid, _NTO_SIDE_CHANNEL, 0);
        char c = 0;
        MsgSend(coid, &c, 1, NULL, 0);
        _exit(0);
    }
    struct _msg_info info;
    char c;
    rcvid_t rcvid = MsgReceive(chid, &c, 1, &info);
    struct _client_info *ci = NULL;
    for (int i = 0; i < n; i++) {
        a[i].flags = 77;
    }
    ci_err = ConnectClientInfoAble(info.scoid, &ci, 0, a, n) == -1 ? errno : 0;
    ci_flags = ci ? ci->flags : 0xdead;
    if (ci) {
        ClientInfoExtFree(&ci);
    }
    for (int i = 0; i < n; i++) {
        bits[i] = a[i].flags == 1 ? '1' : a[i].flags == 0 ? '0' : '?';
    }
    bits[n] = 0;
    MsgReply(rcvid, 0, NULL, 0);
    waitpid(pid, NULL, 0);
    return bits;
}

static void nothing(void) {}
static void nonroot(void) { setuid(1000); }
static void chrooted(void) { chroot("/proc"); }
static void deny_clockset(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_CLOCKSET, PROCMGR_AID_EOL);
}
static void setuid_range(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_ALLOW | PROCMGR_AOP_SUBRANGE | PROCMGR_AID_SETUID,
                    (uint64_t)1000, (uint64_t)2000, PROCMGR_AID_EOL);
}
static int a_root, a_both, a_nonroot, a_none;
static void deny_root(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | a_root, PROCMGR_AID_EOL);
}
static void allow_none_nonroot(void)
{
    procmgr_ability(0, PROCMGR_ADN_NONROOT | PROCMGR_AOP_ALLOW | a_none, PROCMGR_AID_EOL);
    setuid(1000);
}

static void client_checks(void)
{
    struct _client_able a[4];
    memset(a, 0, sizeof a);
    a[0].ability = PROCMGR_AID_CLOCKSET;
    a[1].ability = PROCMGR_AID_SPAWN;
    a[2].ability = PROCMGR_AID_SETUID;
    a[2].range_lo = a[2].range_hi = 1500;
    a[3].ability = PROCMGR_AID_SETUID;
    a[3].range_lo = 500;
    a[3].range_hi = 1500;
    const char *b = ask(nothing, a, 4);
    check("ConnectClientInfoAble: root has them all, no _NTO_CI_UNABLE", !strcmp(b, "1111") && ci_flags == 0);
    b = ask(deny_clockset, a, 4);
    check("a denied ability 0, and _NTO_CI_UNABLE", !strcmp(b, "0111") && ci_flags == _NTO_CI_UNABLE);
    b = ask(nonroot, a, 4);
    check("non-root: SPAWN only", !strcmp(b, "0100") && ci_flags == _NTO_CI_UNABLE);
    b = ask(setuid_range, a, 4);
    printf("abil2test:   SETUID 1000-2000: %s\n", b);
    check("a subrange must hold the whole range asked", !strcmp(b, "1110"));
    a[0].ability = 9999;
    b = ask(nothing, a, 1);
    check("an unknown ability 0", !strcmp(b, "0") && ci_flags == _NTO_CI_UNABLE);
    ask(nothing, a, 0);
    check("no entries EFAULT", ci_err == EFAULT);
    a[0].ability = PROCMGR_AID_SPAWN;
    ask(chrooted, a, 1);
    printf("abil2test:   chroot()ed client: flags %#x\n", ci_flags);
    check("a chroot()ed client: _NTO_CI_CHROOT", ci_flags == _NTO_CI_CHROOT);
}

static void custom(void)
{
    char name[200];
    int r76 = procmgr_ability_name(76, name, sizeof name);
    check("procnto's own: 76 is iofunc/chown", r76 == 0 && !strcmp(name, "iofunc/chown"));
    procmgr_ability_name(79, name, sizeof name);
    check("79 is iofunc/exec", !strcmp(name, "iofunc/exec"));
    errno = 0;
    check("QNX's own abilities have no name (EINVAL)",
          procmgr_ability_name(PROCMGR_AID_CLOCKSET, name, sizeof name) == -1 && errno == EINVAL);

    a_root = procmgr_ability_create("abil2test/root", PROCMGR_ADN_ROOT);
    a_both = procmgr_ability_create("abil2test/both", PROCMGR_ADN_ROOT | PROCMGR_ADN_NONROOT);
    a_nonroot = procmgr_ability_create("abil2test/nonroot", PROCMGR_ADN_NONROOT);
    a_none = procmgr_ability_create("abil2test/none", 0);
    printf("abil2test:   created %#x %#x %#x %#x\n", a_root, a_both, a_nonroot, a_none);
    check("created in order", a_root > 79 && a_both == a_root + 1 && a_nonroot == a_root + 2 && a_none == a_root + 3);
    check("bad flags EINVAL", procmgr_ability_create("abil2test/bad", 1) == -EINVAL);
    check("again with more domains: the same ID",
          procmgr_ability_create("abil2test/root", PROCMGR_ADN_ROOT | PROCMGR_ADN_NONROOT) == a_root &&
              procmgr_ability_create("abil2test/none", PROCMGR_ADN_ROOT) == a_none);
    check("again without one of its domains EEXIST",
          procmgr_ability_create("abil2test/root", PROCMGR_ADN_NONROOT) == -EEXIST &&
              procmgr_ability_create("abil2test/both", PROCMGR_ADN_ROOT) == -EEXIST);
    check("lookup finds it", procmgr_ability_lookup("abil2test/both") == a_both);
    procmgr_ability_name(a_both, name, sizeof name);
    check("procmgr_ability_name()", !strcmp(name, "abil2test/both"));
    check("procmgr_ability_name(): a buffer too small fails",
          procmgr_ability_name(a_both, name, strlen("abil2test/both")) == -1);
    char longname[130];
    memset(longname, 'x', sizeof longname);
    longname[128] = 0;
    check("a name of 128 characters E2BIG", procmgr_ability_lookup(longname) == -E2BIG);

    struct _client_able a[4];
    memset(a, 0, sizeof a);
    a[0].ability = a_root;
    a[1].ability = a_both;
    a[2].ability = a_nonroot;
    a[3].ability = a_none;
    const char *b = ask(nothing, a, 4);
    check("root has those created for root", !strcmp(b, "1100"));
    b = ask(nonroot, a, 4);
    check("non-root has those created for non-root", !strcmp(b, "0110"));
    b = ask(deny_root, a, 4);
    check("denied", !strcmp(b, "0100"));
    b = ask(allow_none_nonroot, a, 4);
    check("allowed", !strcmp(b, "0111"));

    int lookup_made = procmgr_ability_lookup("abil2test/looked-up");
    a[0].ability = lookup_made;
    b = ask(nothing, a, 1);
    check("made by a lookup: nobody's until created", lookup_made > a_none && !strcmp(b, "0"));
    check("then created with its domains", procmgr_ability_create("abil2test/looked-up", PROCMGR_ADN_ROOT) == lookup_made);
    b = ask(nothing, a, 1);
    check("root's now", !strcmp(b, "1"));

    pid_t p = fork();
    if (p == 0) {
        procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_ABLE_CREATE, PROCMGR_AID_EOL);
        int t = procmgr_ability_lookup("abil2test/later");
        int c = procmgr_ability_create("abil2test/later", PROCMGR_ADN_ROOT);
        int l = procmgr_ability_lookup("abil2test/root");
        int s = procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | t, PROCMGR_AID_EOL);
        printf("abil2test:   without ABLE_CREATE: lookup %#x, create %d, existing %#x, procmgr_ability %d\n", t, c, l,
               s);
        _exit(t == 0x100000 && c == -EPERM && l == a_root && s == EINVAL ? 0 : 1);
    }
    int st;
    waitpid(p, &st, 0);
    check("without ABLE_CREATE: a temporary ID, EPERM, existing found, temporary refused",
          WIFEXITED(st) && WEXITSTATUS(st) == 0);
}

int main(void)
{
    chid = ChannelCreate(0);
    client_checks();
    custom();
    printf("abil2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

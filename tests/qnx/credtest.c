/* User and group IDs: root drops to another user as sshd does
 * (setgroups(), setgid(), setuid()), seteuid() and back through the
 * saved ID, what an unprivileged process may not do, and a child's
 * inheritance. Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <grp.h>
#include <stdio.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("credtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

int main(void)
{
    check("root", getuid() == 0 && geteuid() == 0 && getgid() == 0);
    check("seteuid", seteuid(15) == 0 && geteuid() == 15 && getuid() == 0);
    check("back through the saved ID", seteuid(0) == 0 && geteuid() == 0);

    gid_t g[2] = {6, 7};
    gid_t got[8];
    check("setgroups", setgroups(2, g) == 0);
    check("getgroups", getgroups(8, got) == 2 && got[0] == 6 && got[1] == 7);
    struct _client_info info[2];
    check("ConnectClientInfo",
          ConnectClientInfo(-1, info, 2) == 0 && info->cred.ngroups == 2 && info->cred.grouplist[1] == 7);

    check("setgid", setgid(6) == 0 && getgid() == 6 && getegid() == 6);
    check("setuid", setuid(15) == 0 && getuid() == 15 && geteuid() == 15);
    check("no way back", setuid(0) == -1 && errno == EPERM && seteuid(0) == -1);
    check("no setgroups", setgroups(1, g) == -1 && errno == EPERM);

    pid_t pid = fork();
    if (pid == 0)
        _exit(getuid() == 15 && getgid() == 6 ? 0 : 1);
    int status = -1;
    waitpid(pid, &status, 0);
    check("a child inherits them", WIFEXITED(status) && WEXITSTATUS(status) == 0);

    printf("credtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* The /compat/qnx overlay as a QNX program sees it: its own tools at the
 * usual paths, the system's for everything else. OsCraft's; QNX has none. */
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("overlaytest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

int main(void)
{
    struct stat a, b;
    check("its own /bin/ksh", stat("/bin/ksh", &a) == 0);
    check("the same as /compat/qnx/bin/ksh", stat("/compat/qnx/bin/ksh", &b) == 0 && a.st_ino == b.st_ino);
    check("its own libc", access("/lib/libc.so.6", R_OK) == 0);
    check("system() runs its own sh", system("pidin -p overlaytest >/dev/null") == 0);
    check("/dev/null is the system's", open("/dev/null", O_RDWR) >= 0);
    check("/tmp is the system's", open("/tmp/overlaytest", O_RDWR | O_CREAT, 0600) >= 0);
    check("found at its real path too", access("/usr/tmp/overlaytest", F_OK) == 0);
    unlink("/tmp/overlaytest");
    check("a missing name stays missing", access("/bin/no-such-program", F_OK) != 0);
    printf("overlaytest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

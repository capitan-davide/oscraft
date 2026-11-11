/* A program's interpreter named through a link (#326): this program is
 * linked with /usr/lib/ld-oscraft.so.1 as its interpreter, which the
 * image links to /usr/lib/ldqnx-64.so.2. procnto opens the interpreter
 * as open() would, following the link (measured): the program starts,
 * and AT_INTP_DEVICE and AT_INTP_INODE name the loader the link leads
 * to, not the link. Prints one "ok"/"FAIL" line per check and
 * "interptest: passed". */
#include <stdio.h>
#include <sys/auxv.h>
#include <sys/stat.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("interptest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    struct stat ld, link;
    long dev = -1, ino = -1;
    for (auxv_t *a = _auxv; a->a_type != AT_NULL; a++) {
        if (a->a_type == AT_INTP_DEVICE)
            dev = a->a_un.a_val;
        if (a->a_type == AT_INTP_INODE)
            ino = a->a_un.a_val;
    }
    check("the loader's stat()", stat("/usr/lib/ldqnx-64.so.2", &ld) == 0);
    check("the link's lstat()", lstat("/usr/lib/ld-oscraft.so.1", &link) == 0 && S_ISLNK(link.st_mode));
    check("AT_INTP_DEVICE is the loader's device", dev == (long)ld.st_dev);
    check("AT_INTP_INODE is the loader's inode, not the link's",
          ino == (long)ld.st_ino && ino != (long)link.st_ino);
    printf("interptest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

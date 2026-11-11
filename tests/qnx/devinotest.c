/* Device and inode numbers of server-attached names (review PATHMEM-25):
 * resmgr_devino() fstat()s the connection that attached a name; procnto
 * answers it with st_dev 1 and an inode number of the name's own, stable
 * and distinct per name (measured), and drivers built on io-char (the
 * serial drivers, devc-pty) give those to their devices: stat() of them
 * reports st_dev 1 and distinct non-zero inodes. Prints one "ok"/"FAIL"
 * line per check and "devinotest: passed". */
#include <errno.h>
#include <stdio.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/stat.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("devinotest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    char what[120];
    dispatch_t *dpp = dispatch_create();
    static resmgr_connect_funcs_t cf;
    static resmgr_io_funcs_t io;
    static iofunc_attr_t a[2];
    iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &cf, _RESMGR_IO_NFUNCS, &io);
    ino64_t inos[2] = {0, 0};
    for (int i = 0; i < 2; i++) {
        iofunc_attr_init(&a[i], S_IFCHR | 0666, NULL, NULL);
        int id = resmgr_attach(dpp, NULL, i ? "/dev/devinoB" : "/dev/devinoA", _FTYPE_ANY, 0, &cf, &io, &a[i]);
        dev_t dev = 0;
        errno = 0;
        int r = resmgr_devino(id, &dev, &inos[i]);
        snprintf(what, sizeof what, "resmgr_devino() of an attached name -> %d errno %d, st_dev %#x", r, errno,
                 (unsigned)dev);
        check(what, r == 0 && dev == 1);
        check("  its inode is not 0", inos[i] != 0);
        ino64_t again = 0;
        resmgr_devino(id, &dev, &again);
        check("  and the same when asked again", again == inos[i]);
    }
    check("two names have distinct inodes", inos[0] != inos[1]);
    static const char *devs[] = {"/dev/ptyp0", "/dev/ptyp1", "/dev/ser1"};
    ino_t seen[3];
    for (int i = 0; i < 3; i++) {
        struct stat s;
        int r = stat(devs[i], &s);
        seen[i] = r == 0 ? s.st_ino : 0;
        snprintf(what, sizeof what, "stat(%s): st_dev %#x, inode %s", devs[i], r == 0 ? (unsigned)s.st_dev : 0,
                 r == 0 && s.st_ino ? "not 0" : "0");
        check(what, r == 0 && s.st_dev == 1 && s.st_ino != 0);
    }
    check("the devices' inodes are distinct", seen[0] != seen[1] && seen[1] != seen[2] && seen[0] != seen[2]);
    printf("devinotest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

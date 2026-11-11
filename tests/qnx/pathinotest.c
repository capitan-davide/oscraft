/* The path space's own nodes (the directories servers are attached
 * below, name_attach() names, /proc/mount's directories) keep their
 * identity across attaches and detaches (review item PATHMEM-7): the
 * inode stat() reports for /dev doesn't change when a server attaches
 * a new top-level name, a directory opened before stays the same
 * directory after, and readdir()'s d_ino of such a node is the st_ino
 * of the node itself, as on procnto (whose nodes are its own
 * structures). The kernel used to hand out positions in a list that
 * every attach reorders. Prints one "ok"/"FAIL" line per check and
 * "pathinotest: passed". */
#include <dirent.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("pathinotest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static ino_t ino_of(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0 ? st.st_ino : 0;
}

/* whether `name` is listed by `d` (from its start), and its d_ino */
static int listed(DIR *d, const char *name, ino_t *ino)
{
    rewinddir(d);
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        if (strcmp(e->d_name, name) == 0) {
            *ino = e->d_ino;
            return 1;
        }
    }
    return 0;
}

/* attach a path of our own as a resource manager (no server loop: the
 * name alone is what's measured) */
static int attach(dispatch_t *dpp, const char *path)
{
    static resmgr_connect_funcs_t cfuncs;
    static resmgr_io_funcs_t iofuncs;
    static iofunc_attr_t attrs[4];
    static int n;
    resmgr_attr_t rattr;
    memset(&rattr, 0, sizeof rattr);
    iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &cfuncs, _RESMGR_IO_NFUNCS, &iofuncs);
    iofunc_attr_init(&attrs[n], S_IFNAM | 0666, NULL, NULL);
    return resmgr_attach(dpp, &rattr, path, _FTYPE_ANY, 0, &cfuncs, &iofuncs, &attrs[n++]);
}

int main(void)
{
    dispatch_t *dpp = dispatch_create();
    check("dispatch_create()", dpp != NULL);
    ino_t dev_before = ino_of("/dev");
    check("/dev has an inode", dev_before != 0);
    DIR *dev = opendir("/dev");
    check("opendir(/dev)", dev != NULL);
    ino_t ino;
    check("/dev's listing has shmem", listed(dev, "shmem", &ino));

    int id1 = attach(dpp, "/zz/aa");
    check("resmgr_attach(/zz/aa)", id1 >= 0);
    ino_t zz_before = ino_of("/zz");
    check("/zz exists with an inode", zz_before != 0);
    DIR *zz = opendir("/zz");
    check("opendir(/zz) lists aa", zz != NULL && listed(zz, "aa", &ino));
    check("/dev keeps its inode after the attach", ino_of("/dev") == dev_before);
    check("the /dev opened before still lists shmem", listed(dev, "shmem", &ino));

    int id2 = attach(dpp, "/yy/bb");
    check("resmgr_attach(/yy/bb)", id2 >= 0);
    check("/zz keeps its inode after another top-level attach", ino_of("/zz") == zz_before);
    check("/dev keeps its inode", ino_of("/dev") == dev_before);
    check("the /zz opened before still lists aa, not bb", listed(zz, "aa", &ino) && !listed(zz, "bb", &ino));
    check("the /dev opened before still lists shmem", listed(dev, "shmem", &ino));
    DIR *root = opendir("/");
    ino_t d_zz = 0, d_yy = 0;
    check("/ lists zz and yy", root != NULL && listed(root, "zz", &d_zz) && listed(root, "yy", &d_yy));
    check("d_ino of zz in / is st_ino of /zz", d_zz == ino_of("/zz"));
    check("d_ino of yy in / is st_ino of /yy", d_yy == ino_of("/yy"));
    check("zz and yy have different inodes", d_zz != d_yy);
    /* /zz/aa is our own name, not offered back to us by stat(): its d_ino
     * is the node's, the same from both directory streams */
    ino_t d_zz_aa = 0, d_zz_aa2 = 0;
    DIR *zz2 = opendir("/zz");
    check("d_ino of aa in /zz is a node number, the same from two streams",
          listed(zz, "aa", &d_zz_aa) && d_zz_aa != 0 && zz2 != NULL && listed(zz2, "aa", &d_zz_aa2) && d_zz_aa == d_zz_aa2);
    closedir(zz2);

    check("resmgr_detach(/yy/bb)", resmgr_detach(dpp, id2, 0) == 0);
    check("/zz keeps its inode after the detach", ino_of("/zz") == zz_before);
    check("/dev keeps its inode after the detach", ino_of("/dev") == dev_before);
    check("the /zz opened before still lists aa", listed(zz, "aa", &ino));
    int id3 = attach(dpp, "/yy/bb");
    check("resmgr_attach(/yy/bb) again", id3 >= 0);
    check("/yy comes back with the inode it had", ino_of("/yy") == d_yy);
    resmgr_detach(dpp, id3, 0);
    resmgr_detach(dpp, id1, 0);
    check("/zz is gone after its last detach", ino_of("/zz") == 0 && errno == ENOENT);
    check("/dev keeps its inode at the end", ino_of("/dev") == dev_before);
    closedir(zz);
    closedir(dev);
    closedir(root);
    printf("pathinotest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Pathname resolution among servers, as procnto's path manager does it:
 * - a process's own names don't resolve for it, unless attached with
 *   _RESMGR_FLAG_SELF;
 * - at one path, the newest server is tried first, unless it asks to come
 *   after (_RESMGR_FLAG_AFTER) or before (_RESMGR_FLAG_BEFORE);
 * - the deepest prefix first; a directory server (_RESMGR_FLAG_DIR)
 *   answers below its path, and at its path before procnto's own
 *   path-space directory there.
 * Each server answers with its own inode (fstat()).
 * And mkdir(), rmdir(), readlink() around the path space and the image
 * filesystem, with procnto's errors. Prints one "ok"/"FAIL" line per check
 * and "pathtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdio.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/stat.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("pathtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

struct srv {
    dispatch_t *dp;
    resmgr_connect_funcs_t cf;
    resmgr_io_funcs_t io;
    iofunc_attr_t attr;
    int started;
};
static struct srv S[5];

static void *loop(void *a)
{
    struct srv *s = a;
    dispatch_context_t *ctp = dispatch_context_alloc(s->dp);
    for (;;) {
        ctp = dispatch_block(ctp);
        dispatch_handler(ctp);
    }
    return NULL;
}

/* Server i (inode 100 + i) attached at `path`. */
static int attach(int i, const char *path, int flags, int dir)
{
    struct srv *s = &S[i];
    iofunc_attr_init(&s->attr, (dir ? S_IFDIR : S_IFREG) | 0666, 0, 0);
    s->attr.inode = 100 + i;
    int id = resmgr_attach(s->dp, NULL, path, _FTYPE_ANY, flags | (dir ? _RESMGR_FLAG_DIR : 0), &s->cf, &s->io,
                           &s->attr);
    if (!s->started) {
        s->started = 1;
        pthread_t t;
        pthread_create(&t, NULL, loop, s);
    }
    usleep(5000);
    return id;
}

/* The inode of the server that answers an open of `p`, or -errno. */
static long who(const char *p)
{
    int fd = open(p, O_RDONLY);
    if (fd < 0) {
        return -errno;
    }
    struct stat st;
    fstat(fd, &st);
    close(fd);
    return (long)st.st_ino;
}

/* 0 if `r` is 0, else errno. */
static int err(int r)
{
    return r == 0 ? 0 : errno;
}

static void making(void)
{
    char buf[64];
    check("mkdir of a new name in the path space: ENOSYS",
          err(mkdir("/pt-new", 0755)) == ENOSYS && err(mkdir("/dev/pt-new", 0755)) == ENOSYS &&
              err(mkdir("/pt-new/x", 0755)) == ENOSYS);
    check("below a server that isn't a directory: ENOSYS", err(mkdir("/dev/null/x", 0755)) == ENOSYS);
    check("where a server is attached: ENOENT, the server isn't asked",
          err(mkdir("/dev/null", 0755)) == ENOENT && err(mkdir("/dev/ser1", 0755)) == ENOENT &&
              err(mkfifo("/dev/zero", 0644)) == ENOENT && err(mkdir("/dev/sem", 0755)) == ENOENT);
    check("in /dev/sem: ENOSYS", err(mkdir("/dev/sem/x", 0755)) == ENOSYS);
    check("a path-space or image directory: EEXIST",
          err(mkdir("/", 0755)) == EEXIST && err(mkdir("/dev", 0755)) == EEXIST &&
              err(mkdir("/bin", 0755)) == EEXIST);
    check("procfs's names, /proc/boot too: EROFS",
          err(mkdir("/proc", 0755)) == EROFS && err(mkdir("/proc/boot", 0755)) == EROFS &&
              err(mkdir("/proc/boot/.", 0755)) == EROFS && err(rmdir("/proc/boot")) == EROFS &&
              err(mkdir("/proc/self", 0755)) == EROFS);
    check("below /proc: ENOENT", err(mkdir("/proc/x", 0755)) == ENOENT && err(mkdir("/proc/boot/ksh", 0755)) == ENOENT);
    check("unlinking an image file: EROFS; a device: ENOENT",
          err(unlink("/bin/ls")) == EROFS && err(unlink("/dev/null")) == ENOENT && err(unlink("/pt-new")) == ENOENT);
    name_attach_t *n = name_attach(NULL, "pathtest", 0);
    check("a path-space directory without image entry: EEXIST, ENOTEMPTY, EINVAL",
          err(mkdir("/dev/name/local", 0755)) == EEXIST && err(rmdir("/dev/name/local")) == ENOTEMPTY &&
              readlink("/dev/name/local", buf, sizeof buf) < 0 && errno == EINVAL);
    check("a name_attach() name: EEXIST, ENOSYS, EINVAL; below it: ENOENT",
          err(mkdir("/dev/name/local/pathtest", 0755)) == EEXIST && err(rmdir("/dev/name/local/pathtest")) == ENOSYS &&
              readlink("/dev/name/local/pathtest", buf, sizeof buf) < 0 && errno == EINVAL &&
              err(mkdir("/dev/name/local/pathtest/x", 0755)) == ENOENT);
    name_detach(n, 0);
}

int main(void)
{
    making();
    for (int i = 0; i < 5; i++) {
        S[i].dp = dispatch_create();
        iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &S[i].cf, _RESMGR_IO_NFUNCS, &S[i].io);
    }
    int own = attach(4, "/pt/own", 0, 0);
    check("one's own name doesn't resolve for oneself", who("/pt/own") == -ENOENT);
    resmgr_detach(S[4].dp, own, 0);

    const int SELF = _RESMGR_FLAG_SELF;
    int a = attach(0, "/pt/x", SELF, 0), b = attach(1, "/pt/x", SELF, 0);
    check("_RESMGR_FLAG_SELF: it does", who("/pt/x") > 0);
    check("two at one path: the newest first", who("/pt/x") == 101);
    resmgr_detach(S[1].dp, b, 0);
    b = attach(1, "/pt/x", SELF | _RESMGR_FLAG_AFTER, 0);
    check("_RESMGR_FLAG_AFTER: after the other", who("/pt/x") == 100);
    resmgr_detach(S[1].dp, b, 0);
    b = attach(1, "/pt/x", SELF | _RESMGR_FLAG_BEFORE, 0);
    check("_RESMGR_FLAG_BEFORE: before it", who("/pt/x") == 101);
    int c = attach(2, "/pt", SELF, 1);
    check("a directory server: not before a deeper prefix", who("/pt/x") == 101);
    check("below its path", who("/pt/y") == 102);
    check("at its path, before procnto's path-space directory", who("/pt") == 102);
    int d = attach(3, "/pt/x", SELF | _RESMGR_FLAG_AFTER, 0);
    check("a deeper one that comes after: after the others there", who("/pt/x") == 101);
    resmgr_detach(S[0].dp, a, 0);
    resmgr_detach(S[1].dp, b, 0);
    usleep(5000);
    check("then it answers", who("/pt/x") == 103);
    resmgr_detach(S[3].dp, d, 0);
    usleep(5000);
    check("then the directory server", who("/pt/x") == 102);
    resmgr_detach(S[2].dp, c, 0);
    usleep(5000);
    check("none: ENOENT", who("/pt/x") == -ENOENT && who("/pt") == -ENOENT);
    printf("pathtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

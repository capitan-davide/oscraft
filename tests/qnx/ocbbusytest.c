/* A descriptor number whose procnto file was detached without close()
 * (review item PATHMEM-16), as procnto (measured): procnto keeps that
 * open-file context and never binds another over it. An open of any of
 * its files on that number is EBUSY (/dev/text, /dev/null, /dev/shmem,
 * /proc), before anything is made (a new file isn't created), a dup()
 * onto it too; a missing name is still ENOENT; stat(), access() and
 * unlink() work, and another server's file opens there. When the process
 * ends the context goes: a 64 MiB object a child kept that way is freed
 * once its name is unlinked. The kernel bound
 * the new open over the old context. Prints one "ok"/"FAIL" line per
 * check and "ocbbusytest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

/* procnto's free memory, as /proc's size */
static long long free_mem(void)
{
    struct stat st;
    stat("/proc", &st);
    return st.st_size;
}

static void check(const char *what, int ok)
{
    printf("ocbbusytest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* open() fails with `e` */
static int fails_with(const char *path, int flags, int e)
{
    errno = 0;
    int fd = open(path, flags, 0600);
    if (fd >= 0)
        close(fd);
    return fd == -1 && errno == e;
}

int main(void)
{
    int keep = open("/dev/shmem/ocbbusy.k", O_RDWR | O_CREAT, 0600);
    int s = shm_open("/ocbbusy", O_RDWR | O_CREAT, 0600);
    check("ConnectDetach() of a shm_open() descriptor", ConnectDetach(s) == 0);
    /* the lowest free number is the detached one */
    check("open(/dev/text) on its number: EBUSY", fails_with("/dev/text", O_WRONLY, EBUSY));
    check("open(/dev/null): EBUSY", fails_with("/dev/null", O_WRONLY, EBUSY));
    check("open(/proc/self/as): EBUSY", fails_with("/proc/self/as", O_RDONLY, EBUSY));
    check("open(O_CREAT) of a new /dev/shmem file: EBUSY", fails_with("/dev/shmem/ocbbusy.n", O_RDWR | O_CREAT, EBUSY));
    struct stat st;
    check("and it wasn't made", stat("/dev/shmem/ocbbusy.n", &st) == -1 && errno == ENOENT);
    errno = 0;
    check("shm_open() of another: EBUSY", shm_open("/ocbbusy2", O_RDWR | O_CREAT, 0600) == -1 && errno == EBUSY);
    errno = 0;
    check("dup() of a /dev/shmem descriptor: EBUSY", dup(keep) == -1 && errno == EBUSY);
    check("a missing name: still ENOENT", fails_with("/dev/ocbbusy-none", O_RDONLY, ENOENT));
    check("stat(/dev/null) works", stat("/dev/null", &st) == 0);
    check("access(/dev/null) works", access("/dev/null", W_OK) == 0);
    check("unlink() works", unlink("/dev/shmem/ocbbusy.k") == 0);
    errno = 0;
    check("close() of the detached number: EBADF", close(s) == -1 && errno == EBADF);
    check("open(/dev/null) still EBUSY", fails_with("/dev/null", O_WRONLY, EBUSY));
    int t = open("/dev/ser1", O_RDWR);
    check("another server's file opens on it", t == s);
    check("and then procnto's files open elsewhere", fails_with("/dev/ocbbusy-none", O_RDONLY, ENOENT) &&
                                                       open("/dev/null", O_WRONLY) > s);
    shm_unlink("/ocbbusy");

    /* a child keeps a 64 MiB object's context that way, and ends */
    long long before = free_mem();
    pid_t c = fork();
    if (c == 0) {
        int f = shm_open("/ocbbusy3", O_RDWR | O_CREAT, 0600);
        ftruncate(f, 64 << 20);
        ConnectDetach(f);
        _exit(0);
    }
    int status;
    waitpid(c, &status, 0);
    check("the child gone, its object stays while named", free_mem() < before - (48LL << 20));
    shm_unlink("/ocbbusy3");
    check("and goes with the child: the object freed once unlinked", free_mem() > before - (8LL << 20));

    printf("ocbbusytest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

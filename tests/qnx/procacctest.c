/* Who may open another process's /proc/<pid>/as and ctl (review item
 * PROCMGR-6, measured): the files belong to the process's user (as mode
 * 0600, ctl 0644); an opener who is neither root nor that user gets
 * EPERM for what the mode doesn't give (root's as: nothing; root's ctl:
 * read-only); root opens anyone's; a process opens its own. Prints one
 * "ok"/"FAIL" line per check and "procacctest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("procacctest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* open(path, oflag): 0, or the errno. */
static int try_open(const char *path, int oflag)
{
    errno = 0;
    int fd = open(path, oflag);
    if (fd >= 0) {
        close(fd);
        return 0;
    }
    return errno;
}

static int owner_of(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0 ? (int)st.st_uid : -1;
}

static int mode_of(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0 ? (int)(st.st_mode & 07777) : -1;
}

int main(void)
{
    char as[64], ctl[64], dir[64];
    pid_t me = getpid();
    snprintf(as, sizeof as, "/proc/%d/as", (int)me);
    snprintf(ctl, sizeof ctl, "/proc/%d/ctl", (int)me);
    snprintf(dir, sizeof dir, "/proc/%d", (int)me);
    check("as is mode 0600, ctl 0644, the directory 0555", mode_of(as) == 0600 && mode_of(ctl) == 0644 && mode_of(dir) == 0555);
    check("root opens its own as and ctl, and process 1's", try_open(as, O_RDWR) == 0 && try_open("/proc/1/as", O_RDONLY) == 0 && try_open("/proc/1/ctl", O_RDWR) == 0);
    int p2c[2];
    pipe(p2c);
    pid_t pid = fork();
    if (pid == 0) {
        int f = 0;
        setuid(100);
        f += try_open(as, O_RDONLY) != EPERM;
        f += try_open(as, O_RDWR) != EPERM;
        f += try_open(ctl, O_RDONLY) != 0;
        f += try_open(ctl, O_WRONLY) != EPERM;
        f += try_open(ctl, O_RDWR) != EPERM;
        char own[64];
        snprintf(own, sizeof own, "/proc/%d/as", (int)getpid());
        f += try_open(own, O_RDWR) != 0;
        snprintf(own, sizeof own, "/proc/%d/ctl", (int)getpid());
        f += try_open(own, O_RDWR) != 0;
        snprintf(own, sizeof own, "/proc/%d", (int)getpid());
        f += owner_of(own) != 100;
        /* stays for the parent's look at a uid-100 process */
        char go;
        read(p2c[0], &go, 1);
        _exit(f);
    }
    usleep(200000);
    char other[64];
    snprintf(other, sizeof other, "/proc/%d/as", (int)pid);
    check("a uid-100 process's as belongs to uid 100", owner_of(other) == 100);
    check("root opens a uid-100 process's as and ctl", try_open(other, O_RDWR) == 0);
    snprintf(other, sizeof other, "/proc/%d/ctl", (int)pid);
    check("(ctl too)", try_open(other, O_RDWR) == 0);
    write(p2c[1], "g", 1);
    int st = -1;
    waitpid(pid, &st, 0);
    check("uid 100: EPERM on root's as (read or write) and on root's ctl for writing, root's ctl readable, its own files open, its directory owned by 100",
          WIFEXITED(st) && WEXITSTATUS(st) == 0);
    printf("procacctest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

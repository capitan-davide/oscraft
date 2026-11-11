/* Who may open a shared memory object, procnto's results (measured):
 * another user gets EACCES from open() and shm_open(), its owner gets in,
 * and fchown() changes the owner. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

#define NAME "/dev/shmem/shmpermtest"

static int fails;

static void check(const char *what, int ok)
{
    printf("shmpermtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void as(uid_t uid, int (*body)(void), const char *what)
{
    if (fork() == 0) {
        setuid(uid);
        _exit(body());
    }
    int st;
    wait(&st);
    check(what, WIFEXITED(st) && WEXITSTATUS(st) == 0);
}

static int refused(void)
{
    errno = 0;
    if (open(NAME, O_RDWR) >= 0 || errno != EACCES) {
        return 1;
    }
    if (open(NAME, O_RDONLY) >= 0 || errno != EACCES) {
        return 1;
    }
    return shm_open("/shmpermtest", O_RDWR, 0) >= 0 || errno != EACCES;
}

static int allowed(void)
{
    return open(NAME, O_RDWR) < 0;
}

int main(void)
{
    struct stat st;
    int f = open(NAME, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("created", f >= 0);
    check("fchown()", fchown(f, 1000, 1000) == 0 && fstat(f, &st) == 0 && st.st_uid == 1000);
    as(1001, refused, "another user: EACCES");
    as(1000, allowed, "its owner gets in");
    unlink(NAME);
    printf("shmpermtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Named semaphores (sem_open(), as procnto): the name space (/dev/sem),
 * O_CREAT and O_EXCL, the initial value, sharing between opens and
 * processes, sem_unlink(), anonymous ones (SEM_ANON); opening an existing
 * one takes read and write permission in the class that applies (owner,
 * group or a supplementary one, others), root may open any, anyone may
 * unlink one. Prints one "ok"/"FAIL" line per check and "semtest: passed". */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <limits.h>
#include <semaphore.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("semtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int value(sem_t *s)
{
    int v = -1;
    sem_getvalue(s, &v);
    return v;
}

static int listed(const char *name)
{
    DIR *d = opendir("/dev/sem");
    if (d == NULL) {
        return -1;
    }
    int found = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        found |= !strcmp(e->d_name, name);
    }
    closedir(d);
    return found;
}

/* Whether uid:gid (with supplementary group `extra`, 0 for none) may open
 * a semaphore made by owner ouid:ogid with `mode`. */
static int may_open(int mode, uid_t ouid, gid_t ogid, uid_t uid, gid_t gid, gid_t extra)
{
    sem_unlink("/semtest-mode");
    pid_t c = fork();
    if (c == 0) {
        setgid(ogid);
        setuid(ouid);
        umask(0);
        _exit(sem_open("/semtest-mode", O_CREAT | O_EXCL, mode, 0) == SEM_FAILED);
    }
    int st;
    waitpid(c, &st, 0);
    pid_t p = fork();
    if (p == 0) {
        gid_t g[1] = {extra};
        setgroups(extra ? 1 : 0, g);
        setgid(gid);
        setuid(uid);
        if (sem_open("/semtest-mode", 0) != SEM_FAILED) {
            _exit(0);
        }
        _exit(errno == EACCES ? 1 : 2);
    }
    waitpid(p, &st, 0);
    sem_unlink("/semtest-mode");
    return WIFEXITED(st) ? WEXITSTATUS(st) == 0 : -1;
}

static void modes(void)
{
    check("others: read and write needed (0606 yes, 0604 and 0644 no)",
          may_open(0606, 0, 0, 100, 100, 0) == 1 && may_open(0604, 0, 0, 100, 100, 0) == 0 &&
              may_open(0644, 0, 0, 100, 100, 0) == 0);
    check("the group's class, a supplementary group's too",
          may_open(0660, 0, 50, 100, 50, 0) == 1 && may_open(0606, 0, 50, 100, 50, 0) == 0 &&
              may_open(0660, 0, 50, 100, 100, 50) == 1);
    check("the owner's class", may_open(0600, 100, 100, 100, 100, 0) == 1 && may_open(0066, 100, 100, 100, 100, 0) == 0);
    check("root opens any", may_open(0000, 100, 100, 0, 0, 0) == 1);
    sem_t *s = sem_open("/semtest-mode", O_CREAT, 0600, 0);
    pid_t p = fork();
    if (p == 0) {
        setuid(100);
        _exit(sem_unlink("/semtest-mode") != 0);
    }
    int st;
    waitpid(p, &st, 0);
    check("anyone may unlink one", s != SEM_FAILED && WIFEXITED(st) && WEXITSTATUS(st) == 0);
    sem_close(s);
}

int main(void)
{
    sem_unlink("/semtest");
    errno = 0;
    sem_t *s = sem_open("/semtest", 0);
    check("sem_open() of a missing name: ENOENT", s == SEM_FAILED && errno == ENOENT);
    s = sem_open("/semtest", O_CREAT, 0640, 3);
    check("sem_open(O_CREAT) with value 3", s != SEM_FAILED && value(s) == 3);

    int l = listed("semtest");
    struct stat st;
    int sr = stat("/dev/sem/semtest", &st);
    printf("semtest:   listed %d, stat %d mode %#o size %lld\n", l, sr, sr == 0 ? (unsigned)st.st_mode : 0,
           sr == 0 ? (long long)st.st_size : 0);
    check("the name is in /dev/sem", l == 1 && sr == 0);

    errno = 0;
    sem_t *x = sem_open("/semtest", O_CREAT | O_EXCL, 0640, 0);
    check("sem_open(O_CREAT | O_EXCL) of an existing name: EEXIST", x == SEM_FAILED && errno == EEXIST);
    sem_t *t = sem_open("/semtest", O_CREAT, 0640, 9);
    check("sem_open(O_CREAT) of an existing name: the same semaphore", t != SEM_FAILED && value(t) == 3);
    sem_post(t);
    check("a post through one open shows in the other", value(s) == 4);
    sem_close(t);

    errno = 0;
    x = sem_open("/semtest2", O_CREAT, 0600, (unsigned)SEM_VALUE_MAX + 1);
    check("a value above SEM_VALUE_MAX: EINVAL", x == SEM_FAILED && errno == EINVAL);

    pid_t pid = fork();
    if (pid == 0) {
        sem_t *c = sem_open("/semtest", 0);
        for (int i = 0; i < 4; i++) {
            sem_wait(c);
        }
        /* Blocks until the parent posts. */
        sem_wait(c);
        _exit(value(c) == 0 ? 0 : 1);
    }
    usleep(100000);
    sem_post(s);
    int status;
    waitpid(pid, &status, 0);
    check("another process waits on it and gets the parent's post", WIFEXITED(status) && WEXITSTATUS(status) == 0);

    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    ts.tv_nsec += 20000000;
    if (ts.tv_nsec >= 1000000000) {
        ts.tv_sec++;
        ts.tv_nsec -= 1000000000;
    }
    errno = 0;
    int r = sem_timedwait(s, &ts);
    check("sem_timedwait() on an empty one: ETIMEDOUT", r == -1 && errno == ETIMEDOUT);
    errno = 0;
    r = sem_trywait(s);
    check("sem_trywait() on an empty one: EAGAIN", r == -1 && errno == EAGAIN);

    check("sem_unlink()", sem_unlink("/semtest") == 0);
    errno = 0;
    r = sem_unlink("/semtest");
    check("sem_unlink() again: ENOENT", r == -1 && errno == ENOENT);
    check("unlinked: gone from /dev/sem", listed("semtest") == 0);
    sem_post(s);
    check("an unlinked one still works for those that have it", value(s) == 1);
    t = sem_open("/semtest", O_CREAT, 0640, 0);
    check("the name again: a new semaphore", t != SEM_FAILED && value(t) == 0);
    sem_close(t);
    sem_unlink("/semtest");
    check("sem_close()", sem_close(s) == 0);
    errno = 0;
    r = sem_destroy(s);
    printf("semtest:   sem_destroy() after close: %d %d\n", r, errno);

    s = sem_open(SEM_ANON, O_CREAT, 0600, 2);
    sem_t *s2 = sem_open(SEM_ANON, O_CREAT, 0600, 5);
    check("SEM_ANON: separate semaphores", s != SEM_FAILED && s2 != SEM_FAILED && value(s) == 2 && value(s2) == 5);
    check("SEM_ANON: not in /dev/sem", listed("__anon__") == 0);
    pid = fork();
    if (pid == 0) {
        sem_post(s);
        _exit(0);
    }
    waitpid(pid, &status, 0);
    check("SEM_ANON: shared with a forked child", value(s) == 3);
    sem_close(s);
    sem_close(s2);

    errno = 0;
    s = sem_open("/semtest", O_CREAT, 0, 0);
    printf("semtest:   mode 0: %s %d\n", s == SEM_FAILED ? "failed" : "opened", errno);
    if (s != SEM_FAILED) {
        sem_close(s);
        sem_unlink("/semtest");
    }

    modes();
    printf("semtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

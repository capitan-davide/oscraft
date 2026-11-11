/* posix_spawn() attributes and file actions (as procnto):
 * - POSIX_SPAWN_SETCRED: the child's user and group IDs; needs
 *   PROCMGR_AID_SPAWN_SETUID (and _SETGID);
 * - POSIX_SPAWN_RESETIDS: the child's effective IDs are the parent's
 *   real ones;
 * - POSIX_SPAWN_SETCWD: the child's working directory;
 * - a close action on a descriptor past the limit fails the spawn with
 *   EBADF (review item PROCMGR-1);
 * - an open file action on a file of /tmp, the child's stdout;
 * - open file actions on another server's file (a pty of devc-pty),
 *   with dup2 and close after them, and failing ones;
 * - POSIX_SPAWN_SETSTACKMAX: the main thread's stack size;
 * - spawn() with SPAWN_EXEC from a process with threads: only the new
 *   program's thread is left, and the fd map is ignored (procnto);
 * - spawn()'s fd map: the child gets the mapped descriptors only, or all
 *   (but close-on-exec ones) with no map; posix_spawn() with dup2
 *   actions keeps all of them.
 * The child (`spawn2test child`) reports through its stdout. Prints one
 * "ok"/"FAIL" line per check and "spawn2test: passed". */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <devctl.h>
#include <process.h>
#include <pthread.h>
#include <sys/procfs.h>
#include <sys/procmgr.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;
static char *self;

static void check(const char *what, int ok)
{
    printf("spawn2test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* Spawn `self child` with `attr` and `fa` (plus stdout to a pipe unless
 * `fa` is given); what it printed, in `out`. Returns posix_spawn()'s
 * result. */
static int run(posix_spawnattr_t *attr, posix_spawn_file_actions_t *fa, char *out, size_t len)
{
    int p[2];
    pipe(p);
    posix_spawn_file_actions_t own;
    if (fa == NULL) {
        posix_spawn_file_actions_init(&own);
        posix_spawn_file_actions_adddup2(&own, p[1], 1);
        fa = &own;
    }
    char *argv[] = { self, "child", NULL };
    pid_t pid;
    int r = posix_spawnp(&pid, self, fa, attr, argv, NULL);
    close(p[1]);
    out[0] = 0;
    if (r == 0) {
        ssize_t n = read(p[0], out, len - 1);
        out[n > 0 ? n : 0] = 0;
        waitpid(pid, NULL, 0);
    }
    close(p[0]);
    return r;
}

/* posix_spawn()'s result for `self quiet` with `fa`. */
static int quiet(posix_spawn_file_actions_t *fa)
{
    char *argv[] = { self, "quiet", NULL };
    pid_t pid;
    int r = posix_spawnp(&pid, self, fa, NULL, argv, NULL);
    if (r == 0) {
        waitpid(pid, NULL, 0);
    }
    return r;
}

/* The calling process's thread count and its main thread's stack size. */
static void self_info(int *threads, unsigned long *stack)
{
    int fd = open("/proc/self/as", O_RDONLY);
    procfs_info i;
    memset(&i, 0, sizeof i);
    devctl(fd, DCMD_PROC_INFO, &i, sizeof i, NULL);
    procfs_status st;
    memset(&st, 0, sizeof st);
    st.tid = 1;
    devctl(fd, DCMD_PROC_TIDSTATUS, &st, sizeof st, NULL);
    close(fd);
    *threads = i.num_threads;
    *stack = st.stksize;
}

static void *sleeper(void *arg)
{
    (void)arg;
    for (;;) {
        sleep(10);
    }
    return NULL;
}

static int setcred_denied(void)
{
    procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AID_SPAWN_SETUID, PROCMGR_AID_EOL);
    posix_spawnattr_t a;
    posix_spawnattr_init(&a);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETCRED);
    posix_spawnattr_setcred(&a, 1234, 0);
    char out[128];
    return run(&a, NULL, out, sizeof out);
}

/* Descriptors 0..7 open (and not close-on-exec), as a bitmask. */
static int fd_mask(void)
{
    int mask = 0;
    for (int fd = 0; fd < 8; fd++) {
        int f = fcntl(fd, F_GETFD);
        if (f != -1 && !(f & FD_CLOEXEC)) {
            mask |= 1 << fd;
        }
    }
    return mask;
}

/* The descriptors `self fds` started with, from its exit status. */
static int child_fds(pid_t pid)
{
    int st;
    if (pid == -1 || waitpid(pid, &st, 0) != pid || !WIFEXITED(st)) {
        return -1;
    }
    return WEXITSTATUS(st);
}

int main(int argc, char **argv)
{
    self = argv[0];
    if (argc == 2 && !strcmp(argv[1], "quiet")) {
        return 0;
    }
    if (argc == 2 && !strcmp(argv[1], "info")) {
        int threads;
        unsigned long stack;
        self_info(&threads, &stack);
        printf("threads %d stack %#lx fd3 %s\n", threads, stack, fcntl(3, F_GETFD) < 0 ? "closed" : "open");
        return 0;
    }
    if (argc == 2 && !strcmp(argv[1], "fds")) {
        return fd_mask();
    }
    if (argc == 2 && !strcmp(argv[1], "child")) {
        char cwd[PATH_MAX];
        printf("uid %d euid %d gid %d egid %d cwd %s\n", (int)getuid(), (int)geteuid(), (int)getgid(),
               (int)getegid(), getcwd(cwd, sizeof cwd) ? cwd : "?");
        return 0;
    }
    char out[256];
    posix_spawnattr_t a;

    posix_spawnattr_init(&a);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETCRED);
    posix_spawnattr_setcred(&a, 1234, 567);
    int r = run(&a, NULL, out, sizeof out);
    printf("spawn2test:   SETCRED: %d: %s", r, out);
    check("POSIX_SPAWN_SETCRED: the child's IDs", r == 0 && strstr(out, "uid 1234 euid 1234 gid 567 egid 567"));

    pid_t pid = fork();
    if (pid == 0) {
        _exit(setcred_denied());
    }
    int st;
    waitpid(pid, &st, 0);
    check("POSIX_SPAWN_SETCRED without SPAWN_SETUID: EPERM", WIFEXITED(st) && WEXITSTATUS(st) == EPERM);

    pid = fork();
    if (pid == 0) {
        /* Real 0, effective 1000: RESETIDS gives the child effective 0. */
        setreuid(0, 1000);
        posix_spawnattr_t b;
        posix_spawnattr_init(&b);
        posix_spawnattr_setxflags(&b, POSIX_SPAWN_RESETIDS);
        char o[256];
        int rr = run(&b, NULL, o, sizeof o);
        printf("spawn2test:   RESETIDS: %d: %s", rr, o);
        _exit(rr == 0 && strstr(o, "uid 0 euid 0") ? 0 : 1);
    }
    waitpid(pid, &st, 0);
    check("POSIX_SPAWN_RESETIDS: the effective IDs are the real ones", WIFEXITED(st) && WEXITSTATUS(st) == 0);

    posix_spawnattr_init(&a);
    int dir = open("/proc/boot", O_RDONLY | O_DIRECTORY);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETCWD);
    posix_spawnattr_setcwd_np(&a, dir);
    r = run(&a, NULL, out, sizeof out);
    printf("spawn2test:   SETCWD: %d: %s", r, out);
    check("POSIX_SPAWN_SETCWD: the child's working directory", r == 0 && strstr(out, "cwd /proc/boot"));
    close(dir);
    dir = open("/proc/boot", O_RDONLY);
    posix_spawnattr_setcwd_np(&a, dir);
    r = run(&a, NULL, out, sizeof out);
    check("POSIX_SPAWN_SETCWD with a directory opened without O_DIRECTORY: ENOTDIR", r == ENOTDIR);
    close(dir);

    posix_spawn_file_actions_t fa;
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 1, "/tmp/spawn2test.out", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    posix_spawnattr_init(&a);
    r = run(&a, &fa, out, sizeof out);
    char file[256] = { 0 };
    int fd = open("/tmp/spawn2test.out", O_RDONLY);
    if (fd >= 0) {
        read(fd, file, sizeof file - 1);
        close(fd);
    }
    unlink("/tmp/spawn2test.out");
    printf("spawn2test:   open file action: %d: %s", r, file);
    check("an open file action on a file of /tmp", r == 0 && strstr(file, "uid 0"));

    /* Another server's file: the pty's slave, opened at 5, dup2()ed to
     * stdout, closed at 5. What the child prints arrives at the master. */
    int master = open("/dev/ptyp2", O_RDWR);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 5, "/dev/ttyp2", O_WRONLY | O_NOCTTY, 0);
    posix_spawn_file_actions_adddup2(&fa, 5, 1);
    posix_spawn_file_actions_addclose(&fa, 5);
    char *cargv[] = { self, "child", NULL };
    pid_t cpid;
    r = posix_spawnp(&cpid, self, &fa, &a, cargv, NULL);
    file[0] = 0;
    if (r == 0) {
        waitpid(cpid, NULL, 0);
        ssize_t n = read(master, file, sizeof file - 1);
        file[n > 0 ? n : 0] = 0;
    }
    printf("spawn2test:   pty file action: %d: %s", r, file);
    check("open, dup2 and close file actions on another server's file", r == 0 && strstr(file, "uid 0"));
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 1, "/dev/ptyp2", O_RDWR, 0);
    r = quiet(&fa);
    check("an open file action the server fails (a pty master open already): EBUSY", r == EBUSY);
    close(master);

    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 1, "/dev/ttyp2/x", O_WRONLY, 0);
    r = quiet(&fa);
    check("an open file action below a pty: ENOENT", r == ENOENT);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 1, "/dev/nosuchfile", O_WRONLY, 0);
    r = quiet(&fa);
    check("an open file action on a file no server has: ENOENT", r == ENOENT);

    /* The main thread's stack (procnto counts a guard page in too). */
    posix_spawnattr_init(&a);
    posix_spawnattr_setxflags(&a, POSIX_SPAWN_SETSTACKMAX);
    posix_spawnattr_setstackmax(&a, 64 * 1024);
    int p[2];
    pipe(p);
    posix_spawn_file_actions_t sfa;
    posix_spawn_file_actions_init(&sfa);
    posix_spawn_file_actions_adddup2(&sfa, p[1], 1);
    char *iargv[] = { self, "info", NULL };
    pid_t ipid;
    posix_spawnp(&ipid, self, &sfa, &a, iargv, NULL);
    close(p[1]);
    ssize_t n = read(p[0], out, sizeof out - 1);
    out[n > 0 ? n : 0] = 0;
    waitpid(ipid, NULL, 0);
    close(p[0]);
    unsigned long stack = 0;
    char *at = strstr(out, "stack ");
    if (at) {
        stack = strtoul(at + 6, NULL, 16);
    }
    printf("spawn2test:   SETSTACKMAX 64K: %s", out);
    check("POSIX_SPAWN_SETSTACKMAX: the main stack's size", stack >= 0x10000 && stack <= 0x11000);

    /* spawn()'s fd map against posix_spawn()'s dup2 actions. */
    int nul = open("/dev/null", O_RDWR);
    int zero = open("/dev/zero", O_RDONLY);
    int d1 = dup(nul), d2 = dup(nul);
    int all = fd_mask();
    char *fargv[] = { self, "fds", NULL };
    int map3[3] = { nul, d1, d2 };
    int got = child_fds(spawnp(self, 3, map3, NULL, fargv, NULL));
    printf("spawn2test:   fds %#x; map of 3 %#x\n", all, got);
    check("spawn(): an fd map of 3, descriptors 0..2 only", got == 0x7);
    int map1[1] = { nul };
    got = child_fds(spawnp(self, 1, map1, NULL, fargv, NULL));
    check("spawn(): an fd map of 1, descriptor 0 only", got == 0x1);
    got = child_fds(spawnp(self, 0, NULL, NULL, fargv, NULL));
    check("spawn(): no fd map, all descriptors", got == all);
    posix_spawn_file_actions_t dfa;
    posix_spawn_file_actions_init(&dfa);
    posix_spawn_file_actions_adddup2(&dfa, nul, 0);
    posix_spawn_file_actions_adddup2(&dfa, d1, 1);
    posix_spawn_file_actions_adddup2(&dfa, d2, 2);
    pid_t fpid = -1;
    posix_spawnp(&fpid, self, &dfa, NULL, fargv, NULL);
    got = child_fds(fpid);
    check("posix_spawn(): dup2 actions keep all descriptors", got == all);
    posix_spawn_file_actions_destroy(&dfa);
    close(nul);
    close(zero);
    close(d1);
    close(d2);

    /* spawn(SPAWN_EXEC) from a process with two more threads, with an fd
     * map that would close descriptor 3. */
    pipe(p);
    pid = fork();
    if (pid == 0) {
        dup2(p[1], 1);
        int keep = open("/dev/null", O_RDONLY);
        if (keep != 3) {
            dup2(keep, 3);
        }
        pthread_t t;
        pthread_create(&t, NULL, sleeper, NULL);
        pthread_create(&t, NULL, sleeper, NULL);
        struct inheritance inh;
        memset(&inh, 0, sizeof inh);
        inh.flags = SPAWN_EXEC;
        int map[4] = { 0, 1, 2, SPAWN_FDCLOSED };
        spawnp(self, 4, map, &inh, iargv, NULL);
        _exit(99);
    }
    close(p[1]);
    n = read(p[0], out, sizeof out - 1);
    out[n > 0 ? n : 0] = 0;
    waitpid(pid, &st, 0);
    close(p[0]);
    printf("spawn2test:   SPAWN_EXEC: %s", out);
    check("SPAWN_EXEC: only the new program's thread", strstr(out, "threads 1 ") != NULL);
    check("SPAWN_EXEC: the fd map is ignored", strstr(out, "fd3 open") != NULL);

    /* a file action naming a descriptor the child can't have: EBADF
     * from the spawn (libc refuses the dup2 and open ones itself, a
     * close only at the spawn; review item PROCMGR-1: the kernel used
     * to size the child's table by it) */
    posix_spawn_file_actions_t big;
    posix_spawn_file_actions_init(&big);
    check("addclose(0x7fffffff) is taken by libc", posix_spawn_file_actions_addclose(&big, 0x7fffffff) == 0);
    check("the spawn fails with EBADF", quiet(&big) == EBADF);
    posix_spawn_file_actions_destroy(&big);

    printf("spawn2test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

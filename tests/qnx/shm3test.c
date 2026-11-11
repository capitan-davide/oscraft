/* Shared memory handles (shm_create_handle() & co.), as procnto serves
 * them:
 * - a handle is for one process and used once: shm_open_handle() or
 *   mmap_handle() of it again is ESRCH, and so is another process's use;
 * - the access asked for must be within the handle's (EACCES); a handle
 *   for more than the descriptor allows can't be made (EACCES);
 * - a SHM_CREATE_HANDLE_OPT_NOFD handle can't be opened (EPERM), only
 *   mapped;
 * - a subrange handle maps only within its range (EINVAL) and at its
 *   offset; its range must be page-aligned (EINVAL) and in the object;
 * - shm_open_handle_pid() checks the handle's creator (ESRCH);
 * - the creator or the recipient may delete a handle;
 * - a child uses the handle its parent made for it;
 * - shm_revoke() (only on a SHMCTL_REVOCABLE object, EINVAL) takes away a
 *   process's mappings of it made since, or everyone's: touching them is
 *   SIGSEGV, SEGV_MAPERR; earlier mappings stay, new ones work; fork
 *   doesn't copy revocable mappings.
 * Prints one "ok"/"FAIL" line per check and "shm3test: passed". */
#include <errno.h>
#include <fcntl.h>
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("shm3test: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static sigjmp_buf jb;
static volatile int fault_code;

static void on_segv(int sig, siginfo_t *si, void *ctx)
{
    (void)sig;
    (void)ctx;
    fault_code = si->si_code;
    siglongjmp(jb, 1);
}

/* Whether reading `p` faults with SIGSEGV, SEGV_MAPERR. */
static int unmapped(volatile char *p)
{
    fault_code = 0;
    if (sigsetjmp(jb, 1) == 0) {
        volatile char c = *p;
        (void)c;
        return 0;
    }
    return fault_code == SEGV_MAPERR;
}

int main(void)
{
    pid_t me = getpid();
    int fd = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    ftruncate(fd, 4 * 4096);
    char *p = mmap(NULL, 4 * 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    for (int i = 0; i < 4; i++) {
        p[i * 4096] = 'a' + i;
    }

    shm_handle_t h;
    check("shm_create_handle()", shm_create_handle(fd, me, O_RDWR, &h, 0) == 0);
    int f2 = shm_open_handle(h, O_RDWR);
    char *q = f2 >= 0 ? mmap(NULL, 4096, PROT_READ, MAP_SHARED, f2, 4096) : MAP_FAILED;
    check("shm_open_handle(): the same object", q != MAP_FAILED && q[0] == 'b');
    errno = 0;
    check("the handle again: ESRCH", shm_open_handle(h, O_RDWR) == -1 && errno == ESRCH);

    shm_create_handle(fd, me, O_RDONLY, &h, 0);
    errno = 0;
    check("more access than the handle's: EACCES", shm_open_handle(h, O_RDWR) == -1 && errno == EACCES);
    errno = 0;
    check("... and the handle is used up", shm_open_handle(h, O_RDONLY) == -1 && errno == ESRCH);

    int ro = open("/dev/shmem/shm3test_ro", O_RDWR | O_CREAT, 0600);
    close(ro);
    ro = open("/dev/shmem/shm3test_ro", O_RDONLY);
    errno = 0;
    check("a handle for more than the descriptor's access: EACCES",
          shm_create_handle(ro, me, O_RDWR, &h, 0) == -1 && errno == EACCES);
    unlink("/dev/shmem/shm3test_ro");

    errno = 0;
    check("unknown options: EINVAL", shm_create_handle(fd, me, O_RDWR, &h, 4) == -1 && errno == EINVAL);
    errno = 0;
    check("for no such process: ESRCH", shm_create_handle(fd, 999999, O_RDWR, &h, 0) == -1 && errno == ESRCH);

    shm_create_handle(fd, me, O_RDWR, &h, SHM_CREATE_HANDLE_OPT_NOFD);
    errno = 0;
    check("a NOFD handle can't be opened: EPERM", shm_open_handle(h, O_RDWR) == -1 && errno == EPERM);
    shm_create_handle(fd, me, O_RDWR, &h, SHM_CREATE_HANDLE_OPT_NOFD);
    q = mmap_handle(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, h, 2 * 4096);
    check("mmap_handle() of a NOFD handle", q != MAP_FAILED && q[0] == 'c');
    errno = 0;
    check("mmap_handle() of it again: ESRCH",
          mmap_handle(NULL, 4096, PROT_READ, MAP_SHARED, h, 0) == MAP_FAILED && errno == ESRCH);
    shm_create_handle(fd, me, O_RDONLY, &h, 0);
    errno = 0;
    check("mmap_handle() writable with a read-only handle: EACCES",
          mmap_handle(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, h, 0) == MAP_FAILED && errno == EACCES);

    errno = 0;
    check("a subrange not page-aligned: EINVAL",
          shm_create_subrange_handle(fd, me, O_RDWR, &h, 0, 4096, 100) == -1 && errno == EINVAL);
    errno = 0;
    check("a subrange past the end: EINVAL",
          shm_create_subrange_handle(fd, me, O_RDWR, &h, 0, 2 * 4096, 3 * 4096) == -1 && errno == EINVAL);
    shm_create_subrange_handle(fd, me, O_RDWR, &h, 0, 2 * 4096, 4096);
    q = mmap_handle(NULL, 4096, PROT_READ, MAP_SHARED, h, 4096);
    check("a subrange handle maps at its offset", q != MAP_FAILED && q[0] == 'c');
    shm_create_subrange_handle(fd, me, O_RDWR, &h, 0, 2 * 4096, 4096);
    errno = 0;
    check("... and not past its end: EINVAL",
          mmap_handle(NULL, 2 * 4096, PROT_READ, MAP_SHARED, h, 4096) == MAP_FAILED && errno == EINVAL);

    shm_create_handle(fd, me, O_RDWR, &h, 0);
    errno = 0;
    check("shm_open_handle_pid() of another creator: ESRCH",
          shm_open_handle_pid(h, O_RDWR, me + 1) == -1 && errno == ESRCH);
    shm_create_handle(fd, me, O_RDWR, &h, 0);
    check("shm_open_handle_pid() of its creator", shm_open_handle_pid(h, O_RDWR, me) >= 0);

    shm_create_handle(fd, me, O_RDWR, &h, 0);
    check("shm_delete_handle()", shm_delete_handle(h) == 0);
    errno = 0;
    check("a deleted handle: ESRCH", shm_open_handle(h, O_RDWR) == -1 && errno == ESRCH);
    errno = 0;
    check("deleting it again: ESRCH", shm_delete_handle(h) == -1 && errno == ESRCH);

    /* A handle for a child, made after it started. */
    int fds[2], back[2];
    pipe(fds);
    pipe(back);
    pid_t pid = fork();
    if (pid == 0) {
        shm_handle_t ch;
        read(fds[0], &ch, sizeof ch);
        char *m = mmap_handle(NULL, 4096, PROT_READ, MAP_SHARED, ch, 3 * 4096);
        char c = m != MAP_FAILED ? m[0] : 0;
        write(back[1], &c, 1);
        _exit(0);
    }
    shm_handle_t ch;
    shm_create_handle(fd, pid, O_RDONLY, &ch, 0);
    errno = 0;
    check("another process's handle: ESRCH", shm_open_handle(ch, O_RDONLY) == -1 && errno == ESRCH);
    write(fds[1], &ch, sizeof ch);
    char c = 0;
    read(back[0], &c, 1);
    waitpid(pid, NULL, 0);
    check("the child maps the handle made for it", c == 'd');

    shm_create_handle(fd, me, O_RDWR, &h, 0);
    pid = fork();
    if (pid == 0) {
        _exit(shm_delete_handle(h) == -1 && errno == ESRCH ? 0 : 1);
    }
    int status = -1;
    waitpid(pid, &status, 0);
    check("neither creator nor recipient: delete is ESRCH", WIFEXITED(status) && WEXITSTATUS(status) == 0);

    /* Revocation. */
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = on_segv;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGSEGV, &sa, NULL);
    int rv = shm_open(SHM_ANON, O_RDWR | O_CREAT, 0600);
    errno = 0;
    check("shm_revoke() of an object not revocable: EINVAL", shm_revoke(rv, me) == -1 && errno == EINVAL);
    shm_ctl(rv, SHMCTL_ANON, 0, 2 * 4096);
    char *before = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, rv, 0);
    shm_ctl(rv, SHMCTL_REVOCABLE, 0, 0);
    char *own = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, rv, 0);
    own[0] = 'R';
    int down[2], up[2];
    pipe(down);
    pipe(up);
    pid = fork();
    if (pid == 0) {
        sigaction(SIGSEGV, &sa, NULL);
        char r[2];
        r[0] = unmapped(own);
        shm_handle_t rh;
        read(down[0], &rh, sizeof rh);
        char *m = mmap_handle(NULL, 4096, PROT_READ, MAP_SHARED, rh, 0);
        char ok = m != MAP_FAILED && m[0] == 'R';
        write(up[1], &ok, 1);
        read(down[0], &rh, 1);
        r[1] = unmapped(m);
        write(up[1], r, 2);
        _exit(0);
    }
    shm_handle_t rh;
    shm_create_handle(rv, pid, O_RDONLY, &rh, 0);
    write(down[1], &rh, sizeof rh);
    char ok = 0, r[2] = {0, 0};
    read(up[0], &ok, 1);
    check("the child maps the revocable object", ok == 1);
    check("shm_revoke() of the child", shm_revoke(rv, pid) == 0);
    write(down[1], "g", 1);
    read(up[0], r, 2);
    waitpid(pid, NULL, 0);
    check("fork() didn't copy the revocable mapping", r[0] == 1);
    check("the child's mapping is gone: SEGV_MAPERR", r[1] == 1);
    check("ours stays", !unmapped(own));
    errno = 0;
    check("shm_revoke() of no such process: ESRCH", shm_revoke(rv, 999999) == -1 && errno == ESRCH);
    check("shm_revoke() of everyone", shm_revoke(rv, -1) == 0 && unmapped(own));
    check("a mapping from before it was revocable stays", !unmapped(before));
    char *again = mmap(NULL, 4096, PROT_READ, MAP_SHARED, rv, 0);
    check("a new mapping works", again != MAP_FAILED && !unmapped(again));

    printf("shm3test: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

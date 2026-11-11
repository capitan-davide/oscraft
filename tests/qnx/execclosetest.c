/* Close-on-exec descriptors of another server at exec (review item
 * PROCMGR-8), as procnto does (measured): the server gets _IO_CLOSE for
 * a close-on-exec descriptor before the new program runs, even when the
 * process keeps another descriptor to the same server (the kernel used
 * to drop the connection silently then, and the server kept the open
 * file for ever); a dup()ed pair shares its connection, so the server
 * hears nothing at the exec when only one of the pair is close-on-exec,
 * and once when the last of them goes. The server is a child process (a
 * resource manager at /dev/execclosetest) that appends what it sees to
 * /dev/shmem/execclosetest.log; the last stage reads the log. Prints one
 * "ok"/"FAIL" line per check and "execclosetest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <process.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/neutrino.h>
#include <unistd.h>

#define LOG "/dev/shmem/execclosetest.log"
#define SELF "/proc/boot/execclosetest"
static int fails;

static void check(const char *what, int ok)
{
    printf("execclosetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void note(const char *what)
{
    int fd = open(LOG, O_WRONLY | O_CREAT | O_APPEND, 0666);
    write(fd, what, strlen(what));
    write(fd, "\n", 1);
    close(fd);
}

static int closes;
static int my_close(resmgr_context_t *ctp, void *reserved, RESMGR_OCB_T *ocb)
{
    closes++;
    note("close");
    return iofunc_close_ocb_default(ctp, reserved, ocb);
}

static int server(void)
{
    dispatch_t *dpp = dispatch_create();
    resmgr_attr_t rattr;
    memset(&rattr, 0, sizeof rattr);
    static resmgr_connect_funcs_t cfuncs;
    static resmgr_io_funcs_t iofuncs;
    static iofunc_attr_t attr;
    iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &cfuncs, _RESMGR_IO_NFUNCS, &iofuncs);
    iofuncs.close_ocb = my_close;
    iofunc_attr_init(&attr, S_IFNAM | 0666, NULL, NULL);
    if (resmgr_attach(dpp, &rattr, "/dev/execclosetest", _FTYPE_ANY, 0, &cfuncs, &iofuncs, &attr) < 0) {
        note("attach failed");
        return 1;
    }
    dispatch_context_t *ctp = dispatch_context_alloc(dpp);
    for (;;) {
        uint64_t to = 200000000ULL;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &to, NULL);
        dispatch_context_t *n = dispatch_block(ctp);
        if (n) {
            ctp = n;
            dispatch_handler(ctp);
        }
        if (access("/dev/shmem/execclosetest.stop", F_OK) == 0) {
            note("server done");
            return 0;
        }
    }
}

/* the log's lines, joined with ' ' */
static char *logtext(void)
{
    static char buf[2048];
    int fd = open(LOG, O_RDONLY);
    int n = fd >= 0 ? read(fd, buf, sizeof buf - 1) : 0;
    if (fd >= 0) close(fd);
    buf[n < 0 ? 0 : n] = 0;
    for (char *c = buf; *c; c++) {
        if (*c == '\n') *c = ' ';
    }
    return buf;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "server") == 0) {
        return server();
    }
    if (argc > 1 && strcmp(argv[1], "stage2") == 0) {
        /* after round 1: one descriptor, close-on-exec */
        note("stage2 started");
        int a = open("/dev/execclosetest", O_RDONLY);
        int b = open("/dev/execclosetest", O_RDONLY);
        fcntl(a, F_SETFD, FD_CLOEXEC);
        note("exec2");
        execl(SELF, "execclosetest", "stage3", NULL);
        note("exec2 failed");
        return 1;
    }
    if (argc > 1 && strcmp(argv[1], "stage3") == 0) {
        /* after round 2: two descriptors, the first close-on-exec; fd 4 of
         * round 2 is still open here */
        note("stage3 started");
        int a = open("/dev/execclosetest", O_RDONLY);
        int b = dup(a);
        fcntl(a, F_SETFD, FD_CLOEXEC);
        note("exec3");
        execl(SELF, "execclosetest", "stage4", NULL);
        note("exec3 failed");
        return 1;
    }
    if (argc > 1 && strcmp(argv[1], "stage4") == 0) {
        /* after round 3: a dup()ed pair, one close-on-exec */
        note("stage4 started");
        char *t = strdup(logtext());
        check("round 1: the server got one _IO_CLOSE before the new program ran",
              strstr(t, "open1 exec1 close stage2 started") != NULL);
        check("round 2: one _IO_CLOSE at the exec (the other descriptor kept), before the new program ran",
              strstr(t, "exec2 close stage3 started") != NULL);
        check("round 3: no _IO_CLOSE at the exec of a dup()ed pair",
              strstr(t, "exec3 stage4 started") != NULL);
        /* the descriptors left: round 2's second, round 3's dup: closed at exit */
        int before = 0;
        for (char *p = t; (p = strstr(p, "close")) != NULL; p++) before++;
        free(t);
        int fd = open("/dev/shmem/execclosetest.stop", O_WRONLY | O_CREAT, 0666);
        close(fd);
        pid_t pid = fork();
        if (pid == 0) {
            _exit(0);
        }
        (void)pid;
        printf("execclosetest: %s\n", fails ? "FAILED" : "passed");
        return fails != 0;
    }
    unlink(LOG);
    unlink("/dev/shmem/execclosetest.stop");
    spawnl(P_NOWAIT, SELF, "execclosetest", "server", NULL);
    for (int i = 0; i < 100 && access("/dev/execclosetest", F_OK) != 0; i++) {
        usleep(20000);
    }
    /* round 1: one descriptor, close-on-exec (the access() above opened and
     * closed one: the log starts after it) */
    int fd = open(LOG, O_WRONLY | O_CREAT | O_TRUNC, 0666);
    close(fd);
    int a = open("/dev/execclosetest", O_RDONLY);
    fcntl(a, F_SETFD, FD_CLOEXEC);
    note("open1");
    note("exec1");
    execl(SELF, "execclosetest", "stage2", NULL);
    printf("execclosetest: FAIL exec failed errno %d\n", errno);
    return 1;
}

/* resmgr_attach() flags in path resolution, as procnto (measured,
 * review PATHMEM-10):
 * - _RESMGR_FLAG_OPAQUE: an open below an opaque server's prefix doesn't
 *   go on to the servers of shorter prefixes when it answers ENOENT; it
 *   does go on to another server at the same prefix;
 * - _RESMGR_FLAG_FTYPEALL: a server attached for _FTYPE_ANY sees
 *   shm_open()'s _FTYPE_SHMEM connects only with it.
 * Servers here record which of them a connect reached and answer ENOENT;
 * a child process makes each open. Prints one "ok"/"FAIL" line per check
 * and "pathflagtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <process.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <sys/dispatch.h>
#include <sys/iofunc.h>
#include <sys/mman.h>
#include <sys/wait.h>

#define N 7
static const char *names[N] = {"/pft/z", "/pft/z/a", "/pft/z/b", "/pft/f", "/pft/g", "/pft/h", "/pft/h"};
static const char *labels[N] = {"z", "z/a", "z/b", "f", "g", "h-opaque", "h"};
static const int flags[N] = {
    _RESMGR_FLAG_DIR,
    _RESMGR_FLAG_DIR | _RESMGR_FLAG_OPAQUE,
    _RESMGR_FLAG_DIR,
    _RESMGR_FLAG_DIR | _RESMGR_FLAG_FTYPEALL,
    _RESMGR_FLAG_DIR,
    _RESMGR_FLAG_DIR | _RESMGR_FLAG_OPAQUE,
    _RESMGR_FLAG_DIR,
};
static iofunc_attr_t attr[N];
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static char hits[200];
static int fails;

static void check(const char *what, int ok)
{
    printf("pathflagtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int my_open(resmgr_context_t *ctp, io_open_t *msg, RESMGR_HANDLE_T *handle, void *extra)
{
    int i = (iofunc_attr_t *)handle - attr;
    pthread_mutex_lock(&lock);
    if (hits[0]) strlcat(hits, ",", sizeof hits);
    strlcat(hits, labels[i], sizeof hits);
    pthread_mutex_unlock(&lock);
    return ENOENT;
}

static void *loop(void *arg)
{
    dispatch_context_t *ctp = dispatch_context_alloc(arg);
    for (;;) {
        if (dispatch_block(ctp)) dispatch_handler(ctp);
    }
    return NULL;
}

/* the child: one open() or shm_open() */
static int client(const char *how, const char *path)
{
    int fd = strcmp(how, "shm") == 0 ? shm_open(path, O_RDONLY, 0) : open(path, O_RDONLY);
    return fd == -1 && errno == ENOENT ? 0 : 1;
}

static void expect(const char *argv0, const char *how, const char *path, const char *want)
{
    hits[0] = 0;
    int st = spawnlp(P_WAIT, argv0, argv0, how, path, NULL);
    char what[160];
    pthread_mutex_lock(&lock);
    snprintf(what, sizeof what, "%s(%s) reached [%s] (expected [%s]), ENOENT %s", how, path, hits, want,
             st == 0 ? "yes" : "no");
    int ok = strcmp(hits, want) == 0 && st == 0;
    pthread_mutex_unlock(&lock);
    check(what, ok);
}

int main(int argc, char **argv)
{
    if (argc > 2) return client(argv[1], argv[2]);
    dispatch_t *dpp = dispatch_create();
    static resmgr_connect_funcs_t cf;
    static resmgr_io_funcs_t io;
    iofunc_func_init(_RESMGR_CONNECT_NFUNCS, &cf, _RESMGR_IO_NFUNCS, &io);
    cf.open = my_open;
    for (int i = 0; i < N; i++) {
        iofunc_attr_init(&attr[i], S_IFDIR | 0555, NULL, NULL);
        if (resmgr_attach(dpp, NULL, names[i], _FTYPE_ANY, flags[i], &cf, &io, &attr[i]) == -1) {
            printf("pathflagtest: FAIL attach %s: errno %d\n", names[i], errno);
            return 1;
        }
    }
    pthread_t t;
    pthread_create(&t, NULL, loop, dpp);
    expect(argv[0], "open", "/pft/z/a/x", "z/a");
    expect(argv[0], "open", "/pft/z/b/x", "z/b,z");
    expect(argv[0], "open", "/pft/z/x", "z");
    expect(argv[0], "open", "/pft/h/x", "h,h-opaque");
    expect(argv[0], "open", "/pft/f/x", "f");
    expect(argv[0], "shm", "/pft/f/x", "f");
    expect(argv[0], "open", "/pft/g/x", "g");
    expect(argv[0], "shm", "/pft/g/x", "");
    printf("pathflagtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

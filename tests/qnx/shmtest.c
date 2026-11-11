/* Shared memory objects: shm_open(), ftruncate(), mmap() shared and
 * private, fork(), read() of what a mapping wrote, shm_unlink(); the
 * descriptor's access mode against the mapping's (review item
 * PATHMEM-3, measured): a read-only descriptor takes no PROT_WRITE
 * with MAP_SHARED but any private mapping; a write-only one maps
 * nothing, PROT_NONE included (EACCES); mprotect() can't make a shared
 * mapping of a read-only descriptor writable (EACCES), a private one it
 * can. Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <utime.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("shmtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

/* mmap() of /acc with `oflag`, `prot` and `flags`: 0, or the errno. */
static int map_as(int oflag, int prot, int flags)
{
    int fd = shm_open("/acc", oflag, 0600);
    if (fd < 0) {
        return errno;
    }
    errno = 0;
    void *p = mmap(NULL, 4096, prot, flags, fd, 0);
    int r = p == MAP_FAILED ? errno : 0;
    if (p != MAP_FAILED) {
        munmap(p, 4096);
    }
    close(fd);
    return r;
}

static void access_modes(void)
{
    int fd = shm_open("/acc", O_RDWR | O_CREAT, 0600);
    ftruncate(fd, 4096);
    close(fd);
    check("O_RDONLY: PROT_READ shared", map_as(O_RDONLY, PROT_READ, MAP_SHARED) == 0);
    check("O_RDONLY: PROT_READ|PROT_WRITE shared: EACCES", map_as(O_RDONLY, PROT_READ | PROT_WRITE, MAP_SHARED) == EACCES);
    check("O_RDONLY: PROT_WRITE shared: EACCES", map_as(O_RDONLY, PROT_WRITE, MAP_SHARED) == EACCES);
    check("O_RDONLY: PROT_READ|PROT_WRITE private", map_as(O_RDONLY, PROT_READ | PROT_WRITE, MAP_PRIVATE) == 0);
    check("O_RDONLY: PROT_NONE shared", map_as(O_RDONLY, PROT_NONE, MAP_SHARED) == 0);
    check("O_RDONLY: PROT_READ|PROT_EXEC shared", map_as(O_RDONLY, PROT_READ | PROT_EXEC, MAP_SHARED) == 0);
    check("O_WRONLY: PROT_READ shared: EACCES", map_as(O_WRONLY, PROT_READ, MAP_SHARED) == EACCES);
    check("O_WRONLY: PROT_WRITE shared: EACCES", map_as(O_WRONLY, PROT_WRITE, MAP_SHARED) == EACCES);
    check("O_WRONLY: PROT_READ|PROT_WRITE private: EACCES", map_as(O_WRONLY, PROT_READ | PROT_WRITE, MAP_PRIVATE) == EACCES);
    check("O_WRONLY: PROT_NONE shared: EACCES", map_as(O_WRONLY, PROT_NONE, MAP_SHARED) == EACCES);
    check("O_RDWR: PROT_READ|PROT_WRITE shared", map_as(O_RDWR, PROT_READ | PROT_WRITE, MAP_SHARED) == 0);
    check("O_RDWR: with PROT_EXEC too", map_as(O_RDWR, PROT_READ | PROT_WRITE | PROT_EXEC, MAP_SHARED) == 0);
    /* the mapping keeps the descriptor's cap: mprotect() can't make a
     * shared mapping of a read-only descriptor writable; a private one
     * it can */
    fd = shm_open("/acc", O_RDONLY, 0);
    char *shared = mmap(NULL, 4096, PROT_READ, MAP_SHARED, fd, 0);
    errno = 0;
    check("O_RDONLY: mprotect(shared, PROT_READ|PROT_WRITE): EACCES",
          shared != MAP_FAILED && mprotect(shared, 4096, PROT_READ | PROT_WRITE) == -1 && errno == EACCES);
    char *priv = mmap(NULL, 4096, PROT_READ, MAP_PRIVATE, fd, 0);
    check("O_RDONLY: mprotect(private, PROT_READ|PROT_WRITE)",
          priv != MAP_FAILED && mprotect(priv, 4096, PROT_READ | PROT_WRITE) == 0 && (priv[0] = 1) == 1);
    if (shared != MAP_FAILED) {
        munmap(shared, 4096);
    }
    if (priv != MAP_FAILED) {
        munmap(priv, 4096);
    }
    close(fd);
    shm_unlink("/acc");
}

int main(void)
{
    const char *name = "/shmtest";
    shm_unlink(name);
    int fd = shm_open(name, O_RDWR | O_CREAT | O_EXCL, 0600);
    check("shm_open", fd >= 0);
    check("ftruncate", ftruncate(fd, 8192) == 0);
    struct stat st;
    check("fstat size", fstat(fd, &st) == 0 && st.st_size == 8192);
    check("a shared memory object", S_ISNAM(st.st_mode));
    int f = open("/tmp/shmtest.file", O_RDWR | O_CREAT | O_TRUNC, 0644);
    check("a file of /tmp is a regular file", f >= 0 && fstat(f, &st) == 0 && S_ISREG(st.st_mode));
    static char big[200 * 1024];
    check("one long write", write(f, big, sizeof big) == sizeof big && fstat(f, &st) == 0 &&
                                st.st_size == sizeof big);
    close(f);
    unlink("/tmp/shmtest.file");

    /* rename(): a file, over another, a directory with what is in it. */
    close(open("/tmp/shm.a", O_WRONLY | O_CREAT, 0644));
    close(open("/tmp/shm.b", O_WRONLY | O_CREAT, 0644));
    check("rename", rename("/tmp/shm.a", "/tmp/shm.c") == 0 && access("/tmp/shm.a", F_OK) == -1 &&
                        access("/tmp/shm.c", F_OK) == 0);
    check("rename over a name", rename("/tmp/shm.c", "/tmp/shm.b") == 0 && access("/tmp/shm.c", F_OK) == -1);
    mkdir("/tmp/shm.d", 0755);
    rename("/tmp/shm.b", "/tmp/shm.d/f");
    check("rename a directory", rename("/tmp/shm.d", "/tmp/shm.e") == 0 && access("/tmp/shm.e/f", F_OK) == 0);
    check("rename into nothing", rename("/tmp/shm.e/f", "/tmp/none/f") == -1 && errno == ENOENT);
    struct utimbuf ut = {1600000000, 1600000000};
    check("utime", utime("/tmp/shm.e/f", &ut) == 0 && stat("/tmp/shm.e/f", &st) == 0 &&
                       st.st_mtime == 1600000000);
    unlink("/tmp/shm.e/f");
    rmdir("/tmp/shm.e");

    char *a = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    char *b = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 4096);
    check("mmap", a != MAP_FAILED && b != MAP_FAILED);

    strcpy(a + 4096, "same pages");
    check("two mappings", strcmp(b, "same pages") == 0);

    pid_t pid = fork();
    if (pid == 0) {
        strcpy(a, "from the child");
        _exit(0);
    }
    int status;
    waitpid(pid, &status, 0);
    check("fork shares", strcmp(a, "from the child") == 0);

    char buf[32] = {0};
    check("read sees mapping", pread(fd, buf, 14, 0) == 14 && strcmp(buf, "from the child") == 0);

    char *c = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd, 0);
    check("private copy", c != MAP_FAILED && strcmp(c, "from the child") == 0);
    if (c != MAP_FAILED) {
        strcpy(c, "private");
    }
    check("private stays private", strcmp(a, "from the child") == 0);

    check("shm_unlink", shm_unlink(name) == 0);
    check("open after unlink fails", shm_open(name, O_RDWR, 0) == -1);
    strcpy(a, "still mapped");
    check("mapping outlives the name", strcmp(a, "still mapped") == 0);

    access_modes();
    printf("shmtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

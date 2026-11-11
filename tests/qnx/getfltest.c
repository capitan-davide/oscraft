/* fcntl(F_GETFL) and F_SETFL on procnto's files (issue #291), as procnto
 * (measured): F_GETFL never reports the flags only the open used
 * (O_CREAT, O_TRUNC, O_EXCL, O_NOCTTY), on any of them; F_SETFL sets
 * O_APPEND, O_NONBLOCK, O_SYNC... on a /dev/shmem file or /proc, and on
 * /dev/null, /dev/zero and /dev/text it answers 0 and changes nothing.
 * The boot script opens its console with O_LARGEFILE. The kernel reported
 * the creation flags, changed the devices' flags, and opened the console
 * without O_LARGEFILE.
 * Prints one "ok"/"FAIL" line per check and "getfltest: passed". */
#include <fcntl.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("getfltest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static int getfl(const char *path, int flags)
{
    int fd = open(path, flags, 0600);
    int f = fcntl(fd, F_GETFL);
    close(fd);
    return f;
}

int main(void)
{
    char line[120];
    const char *paths[] = { "/dev/shmem/getfltest", "/dev/null", "/dev/zero", "/dev/text", "/proc/self/as" };
    for (int i = 0; i < 5; i++) {
        snprintf(line, sizeof line, "%s, O_RDWR|O_CREAT|O_TRUNC|O_NOCTTY: F_GETFL O_RDWR|O_LARGEFILE", paths[i]);
        check(line, getfl(paths[i], O_RDWR | O_CREAT | O_TRUNC | O_NOCTTY) == (O_RDWR | O_LARGEFILE));
    }
    unlink("/dev/shmem/getfltest");
    check("/dev/shmem/getfltest, O_RDWR|O_CREAT|O_EXCL: F_GETFL O_RDWR|O_LARGEFILE",
          getfl("/dev/shmem/getfltest", O_RDWR | O_CREAT | O_EXCL) == (O_RDWR | O_LARGEFILE));
    unlink("/dev/shmem/getfltest");
    int s = shm_open("/getfltest", O_RDWR | O_CREAT, 0600);
    check("shm_open(O_RDWR|O_CREAT): F_GETFL O_RDWR", fcntl(s, F_GETFL) == O_RDWR);
    check("and its dup()", fcntl(dup(s), F_GETFL) == O_RDWR);
    shm_unlink("/getfltest");
    check("F_SETFL(O_APPEND|O_NONBLOCK) on it", fcntl(s, F_SETFL, O_APPEND | O_NONBLOCK) == 0 &&
                                                 fcntl(s, F_GETFL) == (O_RDWR | O_APPEND | O_NONBLOCK));

    for (int i = 1; i < 4; i++) {
        int fd = open(paths[i], O_RDONLY | O_NONBLOCK);
        snprintf(line, sizeof line, "%s: F_SETFL(O_APPEND) answers 0, the flags kept", paths[i]);
        check(line, fcntl(fd, F_SETFL, O_APPEND) == 0 && fcntl(fd, F_GETFL) == (O_NONBLOCK | O_LARGEFILE));
        close(fd);
    }
    int fd = open("/proc/self/as", O_RDONLY);
    check("/proc/self/as: F_SETFL(O_APPEND) sets it",
          fcntl(fd, F_SETFL, O_APPEND) == 0 && fcntl(fd, F_GETFL) == O_APPEND);
    close(fd);

    /* the boot script's console, inherited through the shell */
    check("fd 0 (the boot script's console): F_GETFL has O_LARGEFILE", (fcntl(0, F_GETFL) & O_LARGEFILE) != 0);

    printf("getfltest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

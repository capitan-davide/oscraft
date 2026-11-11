/* Process creation with descriptors set up for the child: popen() (QNX
 * spawn() with an fd map), system(), and posix_spawn() with file actions
 * (dup2, close, open). Prints one "ok"/"FAIL" line per case. With an
 * argument, runs only that case: popen, system, dup2 or open. */
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static int fails;

static void check(const char *what, int ok)
{
    printf("spawntest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

/* Read what the child writes to `fd` until it closes. */
static void slurp(int fd, char *buf, size_t size)
{
    size_t n = 0;
    ssize_t r;
    while (n + 1 < size && (r = read(fd, buf + n, size - 1 - n)) > 0) {
        n += r;
    }
    buf[n] = 0;
}

int main(int argc, char **argv)
{
    char buf[128];
    int status;
    const char *only = argc > 1 ? argv[1] : NULL;
#define CASE(name) if (only == NULL || strcmp(only, name) == 0)

    /* popen(): the child's stdout is the pipe. */
    CASE("popen") {
    FILE *f = popen("echo popen-out", "r");
    buf[0] = 0;
    if (f != NULL) {
        if (fgets(buf, sizeof buf, f) == NULL) {
            buf[0] = 0;
        }
        status = pclose(f);
    }
    check("popen", f != NULL && strcmp(buf, "popen-out\n") == 0);
    }

    /* system() */
    CASE("system") {
    status = system("exit 3");
    check("system", WIFEXITED(status) && WEXITSTATUS(status) == 3);
    }

    int p[2];
    posix_spawn_file_actions_t fa;
    pid_t pid;
    int rc;
    /* posix_spawn() with dup2 and close: the child's stdout is a pipe. */
    CASE("dup2") {
    pipe(p);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_adddup2(&fa, p[1], 1);
    posix_spawn_file_actions_addclose(&fa, p[0]);
    posix_spawn_file_actions_addclose(&fa, p[1]);
    char *argv1[] = {"echo", "dup2-out", NULL};
    rc = posix_spawnp(&pid, "echo", &fa, NULL, argv1, environ);
    posix_spawn_file_actions_destroy(&fa);
    close(p[1]);
    slurp(p[0], buf, sizeof buf);
    close(p[0]);
    if (rc == 0) {
        waitpid(pid, &status, 0);
    }
    check("posix_spawn dup2/close", rc == 0 && strcmp(buf, "dup2-out\n") == 0);
    }

    /* posix_spawn() with open: the child's stdin is a file. */
    CASE("open") {
    pipe(p);
    posix_spawn_file_actions_init(&fa);
    posix_spawn_file_actions_addopen(&fa, 0, "/proc/boot/spawntest.txt", O_RDONLY, 0);
    posix_spawn_file_actions_adddup2(&fa, p[1], 1);
    posix_spawn_file_actions_addclose(&fa, p[0]);
    posix_spawn_file_actions_addclose(&fa, p[1]);
    char *argv2[] = {"cat", NULL};
    rc = posix_spawnp(&pid, "cat", &fa, NULL, argv2, environ);
    posix_spawn_file_actions_destroy(&fa);
    close(p[1]);
    slurp(p[0], buf, sizeof buf);
    close(p[0]);
    if (rc == 0) {
        waitpid(pid, &status, 0);
    }
    check("posix_spawn open", rc == 0 && strcmp(buf, "from a file\n") == 0);
    }

    printf("spawntest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

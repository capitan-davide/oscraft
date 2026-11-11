/* chroot(): paths resolve from the new root (through /tmp, a link to
 * /dev/shmem), ".." stops there, what is outside is gone, and a child
 * inherits it. Prints one "ok"/"FAIL" line per case. */
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
    printf("roottest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static int has(const char *path, const char *text)
{
    char buf[32] = {0};
    int fd = open(path, O_RDONLY);
    if (fd < 0)
        return 0;
    read(fd, buf, sizeof buf - 1);
    close(fd);
    return strcmp(buf, text) == 0;
}

int main(void)
{
    mkdir("/tmp/jail", 0755);
    mkdir("/tmp/jail/d", 0755);
    int fd = open("/tmp/jail/f", O_WRONLY | O_CREAT | O_TRUNC, 0644);
    write(fd, "inside", 6);
    close(fd);

    check("chroot", chroot("/tmp/jail") == 0);
    check("chdir to the root", chdir("/") == 0);
    check("a file at the root", has("/f", "inside"));
    check("relative to the working directory", has("f", "inside"));
    check("\"..\" stops at the root", has("/../../f", "inside"));
    struct stat st;
    check("a directory", stat("/d", &st) == 0 && S_ISDIR(st.st_mode));
    check("outside is gone", open("/proc/boot/roottest", O_RDONLY) == -1 && errno == ENOENT);

    pid_t pid = fork();
    if (pid == 0)
        _exit(has("/f", "inside") ? 0 : 1);
    int status = -1;
    waitpid(pid, &status, 0);
    check("a child inherits the root", WIFEXITED(status) && WEXITSTATUS(status) == 0);

    unlink("/f");
    rmdir("/d");
    printf("roottest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

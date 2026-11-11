/* Files in /tmp that aren't whole programs, spawned (review item
 * PROCMGR-4): a copy of a real program cut short at 100, 300 and 4000
 * bytes (no header, headers past the end, a segment past the end), an
 * empty file and a text file without "#!" are all ENOEXEC, as procnto;
 * the whole copy runs. The loader's arithmetic on header fields is
 * checked too (sizes that overflow, a segment larger in the file than in
 * memory, misaligned offsets: ENOEXEC), which no file here can show.
 * Prints one "ok"/"FAIL" line per check and "badexectest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;
static int fails;

static void check(const char *what, int ok)
{
    printf("badexectest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* /tmp/`path` as the first `len` bytes of `from`, or `text`. */
static void make(const char *path, const char *from, size_t len, const char *text)
{
    int out = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0755);
    if (text) {
        write(out, text, strlen(text));
    } else if (from) {
        static char buf[1 << 20];
        int in = open(from, O_RDONLY);
        ssize_t n = read(in, buf, len < sizeof buf ? len : sizeof buf);
        if (n > 0) {
            write(out, buf, n);
        }
        close(in);
    }
    close(out);
}

/* posix_spawn()'s result for `path`; a child that ran is waited for. */
static int try_spawn(const char *path)
{
    pid_t pid = -1;
    char *argv[] = { (char *)path, NULL };
    int r = posix_spawn(&pid, path, NULL, NULL, argv, environ);
    if (r == 0) {
        int st;
        waitpid(pid, &st, 0);
    }
    return r;
}

int main(void)
{
    const char *prog = "/compat/qnx/usr/bin/toybox";
    struct stat st;
    if (stat(prog, &st) != 0) {
        prog = "/proc/boot/pidin";
    }
    make("/tmp/bx_whole", prog, 1 << 20, NULL);
    make("/tmp/bx_100", prog, 100, NULL);
    make("/tmp/bx_300", prog, 300, NULL);
    make("/tmp/bx_4000", prog, 4000, NULL);
    make("/tmp/bx_empty", NULL, 0, NULL);
    make("/tmp/bx_text", NULL, 0, "hello, not a program\n");
    check("the whole copy runs", try_spawn("/tmp/bx_whole") == 0);
    check("100 bytes (no ELF header): ENOEXEC", try_spawn("/tmp/bx_100") == ENOEXEC);
    check("300 bytes (program headers past the end): ENOEXEC", try_spawn("/tmp/bx_300") == ENOEXEC);
    check("4000 bytes (a segment past the end): ENOEXEC", try_spawn("/tmp/bx_4000") == ENOEXEC);
    check("an empty file: ENOEXEC", try_spawn("/tmp/bx_empty") == ENOEXEC);
    check("a text file without #!: ENOEXEC", try_spawn("/tmp/bx_text") == ENOEXEC);
    unlink("/tmp/bx_whole");
    unlink("/tmp/bx_100");
    unlink("/tmp/bx_300");
    unlink("/tmp/bx_4000");
    unlink("/tmp/bx_empty");
    unlink("/tmp/bx_text");
    printf("badexectest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

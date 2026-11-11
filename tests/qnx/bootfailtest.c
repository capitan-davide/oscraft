/* A boot script program that fails to start (review item PROCMGR-9).
 * The image's boot script runs /proc/boot/notaprogram, a text file:
 * procnto reports the failure and goes on, and no process is left behind.
 * The kernel used to leave the process it had made for the program in the
 * process table for ever, with no thread. Every live process must have a
 * thread: any that has none (and isn't a zombie) is reported, with its
 * pid. Prints one "ok"/"FAIL" line per check and "bootfailtest: passed". */
#include <dirent.h>
#include <devctl.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/procfs.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("bootfailtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

int main(void)
{
    DIR *d = opendir("/proc");
    check("/proc opened", d != NULL);
    if (d == NULL)
        return 1;
    int seen = 0, threadless = 0;
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        char *end;
        long pid = strtol(e->d_name, &end, 10);
        if (*end != '\0' || pid <= 0)
            continue;
        char path[64];
        snprintf(path, sizeof path, "/proc/%ld/as", pid);
        int fd = open(path, O_RDONLY);
        if (fd < 0)
            continue; /* ended meanwhile */
        procfs_info i;
        memset(&i, 0, sizeof i);
        int r = devctl(fd, DCMD_PROC_INFO, &i, sizeof i, NULL);
        close(fd);
        if (r != 0)
            continue;
        seen++;
        if (i.num_threads == 0 && !(i.flags & (_NTO_PF_ZOMBIE | _NTO_PF_TERMING))) {
            printf("bootfailtest: process %ld has no thread (flags %#x)\n", pid, (unsigned)i.flags);
            threadless++;
        }
    }
    closedir(d);
    check("processes listed", seen > 1);
    check("no live process without a thread", threadless == 0);
    printf("bootfailtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Requests to procnto (mmap(), open() of a procnto device) don't fail
 * with EINTR when a signal comes while procnto is busy, as on QNX, where
 * procnto's thread pool receives them at once: 200 children end
 * (SIGCHLD, with a handler) while the parent maps and opens 10000 times.
 * Prints "eintrtest: passed" or the counts. */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/wait.h>
#include <unistd.h>
#include <fcntl.h>
static volatile int chld;
static void on_chld(int s) { (void)s; chld++; }
int main(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_chld;
    sigaction(SIGCHLD, &sa, NULL);
    int maps = 0, eintr_map = 0, eintr_open = 0, opens = 0, other = 0;
    for (int round = 0; round < 200; round++) {
        if (fork() == 0) _exit(0);
        for (int i = 0; i < 50; i++) {
            void *p = mmap(NULL, 65536, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
            if (p == MAP_FAILED) { if (errno == EINTR) eintr_map++; else other++; }
            else { maps++; munmap(p, 65536); }
            int fd = open("/dev/null", O_RDONLY);
            if (fd < 0) { if (errno == EINTR) eintr_open++; else other++; }
            else { opens++; close(fd); }
        }
        while (waitpid(-1, NULL, WNOHANG) > 0) {}
    }
    printf("eintrtest: %d SIGCHLD, mmap %d ok %d EINTR, open %d ok %d EINTR, %d other errors\n", chld, maps, eintr_map, opens, eintr_open, other);
    int ok = eintr_map == 0 && eintr_open == 0 && other == 0;
    printf("eintrtest: %s\n", ok ? "passed" : "FAILED");
    return !ok;
}

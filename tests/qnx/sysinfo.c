/*
 * System information through QNX's libc: uname() and confstr() are
 * answered by procnto (_SYS_CONF), isatty() by the console (_IO_DEVCTL),
 * getpid() comes from the thread-local storage the kernel sets up.
 */
#include <stdio.h>
#include <unistd.h>
#include <sys/utsname.h>

int main(void)
{
    struct utsname u;
    char path[128];

    if (uname(&u) == 0)
        printf("uname: %s %s %s %s %s\n", u.sysname, u.nodename, u.release, u.version, u.machine);
    else
        perror("uname");
    if (confstr(_CS_PATH, path, sizeof path) > 0)
        printf("confstr(_CS_PATH) = %s\n", path);
    printf("isatty(1) = %d, getpid() = %d, sysconf(_SC_PAGESIZE) = %ld\n",
           isatty(1), getpid(), sysconf(_SC_PAGESIZE));
    printf("free pages: %ld\n", sysconf(_SC_AVPHYS_PAGES));
    return 0;
}

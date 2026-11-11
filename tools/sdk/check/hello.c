/* The SDK check: a C program built out of tree with the CMake toolchain
   file, run on OsCraft. */
#include <stdio.h>
#include <unistd.h>
#include <sys/utsname.h>

int
main(void)
{
	struct utsname u;

	uname(&u);
	printf("sdk-c: hello from %s on %s, pid>1:%d\n", u.sysname, u.machine, getpid() > 1);
	return (0);
}

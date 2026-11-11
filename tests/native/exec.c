/*
 * Runs its arguments as a program: exec-test <path> [args...]. For the
 * tests that need a native program from QNX's shell (QNX=1): a QNX
 * program's /bin/sh is its own, found first under /compat/qnx, but this is
 * a native program, so the path it gives execv() is the system's:
 * `exec-test /bin/sh -c ...` is FreeBSD's sh, under its own name.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int
main(int argc, char **argv)
{
	if (argc < 2) {
		fprintf(stderr, "usage: exec-test <path> [args...]\n");
		return (2);
	}
	execv(argv[1], argv + 1);
	fprintf(stderr, "exec-test: %s: %s\n", argv[1], strerror(errno));
	return (127);
}

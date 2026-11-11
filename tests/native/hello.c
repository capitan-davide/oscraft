/* A FreeBSD program on OsCraft, through libsys alone: no libc. It writes
   to its standard output, checks a failed call's errno, and exits with a
   status the shell can see. */
#include <errno.h>
#include <unistd.h>

static size_t
len(const char *s)
{
	size_t n = 0;
	while (s[n] != '\0')
		n++;
	return (n);
}

static void
put(const char *s)
{
	write(1, s, len(s));
}

static void
putnum(unsigned long n)
{
	char buf[24];
	int i = sizeof(buf);
	buf[--i] = '\n';
	do {
		buf[--i] = '0' + n % 10;
		n /= 10;
	} while (n != 0);
	write(1, buf + i, sizeof(buf) - i);
}

int
main(int argc, char *argv[], char *envp[])
{
	int i;

	put("hello-test: a FreeBSD program on OsCraft, argc=");
	putnum(argc);
	for (i = 0; i < argc; i++) {
		put("hello-test: argv: ");
		put(argv[i]);
		put("\n");
	}
	for (i = 0; envp[i] != NULL; i++)
		;
	put("hello-test: envc=");
	putnum(i);
	/* A write on a descriptor that is not open: EBADF, FreeBSD's 9. */
	errno = 0;
	if (write(99, "x", 1) == -1 && errno == EBADF)
		put("hello-test: write(99) -> EBADF\n");
	else {
		put("hello-test: write(99) errno=");
		putnum(errno);
	}
	put("hello-test: getpid=");
	putnum(getpid());
	return (argc == 3 ? 7 : 1);
}

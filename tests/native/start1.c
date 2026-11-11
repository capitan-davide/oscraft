/* What libc's __libc_start1() does, for a program without a libc: run
   main and end with what it returns. crt1 calls it. */
#include <unistd.h>

extern char **environ;
ssize_t _write(int, const void *, size_t);

void
__libc_start1(int argc, char *argv[], char *env[], void (*cleanup)(void),
    int (*mainX)(int, char *[], char *[]))
{
	(void)cleanup;
	environ = env;
	_exit(mainX(argc, argv, env));
}

/* write() is interposed: libc defines it over _write(). Without a libc,
   this does. */
ssize_t
write(int fd, const void *buf, size_t nbytes)
{
	return (_write(fd, buf, nbytes));
}

/* libsys's errno, as FreeBSD's __error.c keeps it; here for a program
   without that file. */
static int libsys_errno;

int *
__error(void)
{
	return (&libsys_errno);
}

/* What FreeBSD's auxv.c and libc give libsys: the auxiliary vector after
   the environment, and memcpy. Here for a program without them. */
#include <string.h>

void *__elf_aux_vector;
extern char **environ;

void
__init_elf_aux_vector(void)
{
	char **p = environ;

	if (__elf_aux_vector != NULL || p == NULL)
		return;
	while (*p++ != NULL)
		;
	__elf_aux_vector = p;
}

int
_elf_aux_info(int aux, void *buf, int buflen)
{
	unsigned long *a;

	__init_elf_aux_vector();
	for (a = __elf_aux_vector; a[0] != 0; a += 2) {
		if ((int)a[0] == aux && buflen == sizeof(int)) {
			*(int *)buf = (int)a[1];
			return (0);
		}
	}
	return (2); /* ENOENT */
}

void *
memcpy(void *d, const void *s, size_t n)
{
	char *dp = d;
	const char *sp = s;

	while (n-- > 0)
		*dp++ = *sp++;
	return (d);
}

void *
memset(void *d, int c, size_t n)
{
	char *dp = d;

	while (n-- > 0)
		*dp++ = (char)c;
	return (d);
}

void *
memmove(void *d, const void *s, size_t n)
{
	char *dp = d;
	const char *sp = s;

	if (dp < sp)
		while (n-- > 0)
			*dp++ = *sp++;
	else
		while (n > 0) {
			n--;
			dp[n] = sp[n];
		}
	return (d);
}

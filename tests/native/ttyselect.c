/* select() on a terminal: a timeout with nothing typed, in canonical
   mode and in raw mode (what top does between refreshes). */
#include <sys/select.h>
#include <sys/time.h>
#include <stdio.h>
#include <termios.h>
#include <time.h>
#include <unistd.h>

static void
wait_once(const char *name)
{
	struct timespec a, b;
	struct timeval tv = { 0, 300000 };
	fd_set r;
	int n;

	FD_ZERO(&r);
	FD_SET(0, &r);
	clock_gettime(CLOCK_MONOTONIC, &a);
	n = select(2, &r, NULL, NULL, &tv);
	clock_gettime(CLOCK_MONOTONIC, &b);
	printf("ttyselect: %s n=%d set=%d ms=%ld\n", name, n, FD_ISSET(0, &r) != 0,
	    (long)((b.tv_sec - a.tv_sec) * 1000 + (b.tv_nsec - a.tv_nsec) / 1000000));
	fflush(stdout);
}

int
main(void)
{
	struct termios old, raw;

	printf("ttyselect: isatty=%d\n", isatty(0));
	wait_once("canon");
	if (tcgetattr(0, &old) == 0) {
		raw = old;
		raw.c_lflag &= ~(ICANON | ECHO);
		raw.c_cc[VMIN] = 1;
		raw.c_cc[VTIME] = 0;
		printf("ttyselect: raw=%d\n", tcsetattr(0, TCSADRAIN, &raw));
		wait_once("raw");
		tcsetattr(0, TCSADRAIN, &old);
	}
	printf("ttyselect: done\n");
	return (0);
}

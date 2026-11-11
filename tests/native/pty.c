/* Pseudo-terminals on OsCraft's native manager (user/pty), by FreeBSD's
   semantics: posix_openpt(), grantpt(), ptsname(), echo and canonical
   input, raw mode, ^C to the foreground group, poll(), the window size,
   and each end's close. "ok"/"FAIL" per check, then "pty-test: passed". */
#include <sys/ioctl.h>
#include <sys/wait.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>

static int fails;

static void
check(const char *what, int ok)
{
	printf("pty-test: %s %s\n", ok ? "ok" : "FAIL", what);
	fflush(stdout);
	fails += !ok;
}

int
main(void)
{
	char buf[128];
	int m = posix_openpt(O_RDWR | O_NOCTTY);
	check("posix_openpt()", m >= 0);
	check("grantpt(), unlockpt()", grantpt(m) == 0 && unlockpt(m) == 0);
	char *name = ptsname(m);
	check("ptsname(): /dev/ttyp0-like", name != NULL && strncmp(name, "/dev/tty", 8) == 0);
	int s = open(name, O_RDWR | O_NOCTTY);
	check("the worker opens", s >= 0);
	check("it is a terminal", isatty(s) == 1);
	errno = 0;
	check("a second open of the controller: EBUSY", open("/dev/ptyp0", O_RDWR) == -1 && errno == EBUSY);

	write(m, "hello\n", 6);
	int n = read(s, buf, sizeof buf);
	check("a line reaches the worker", n == 6 && memcmp(buf, "hello\n", 6) == 0);
	n = read(m, buf, sizeof buf);
	check("echoed, with NL as CR NL", n == 7 && memcmp(buf, "hello\r\n", 7) == 0);
	write(m, "ab\x7f" "c\n", 5);
	n = read(s, buf, sizeof buf);
	check("the erase key edits the line", n == 3 && memcmp(buf, "ac\n", 3) == 0);
	read(m, buf, sizeof buf);
	write(s, "out\n", 4);
	n = read(m, buf, sizeof buf);
	check("output, with ONLCR", n == 5 && memcmp(buf, "out\r\n", 5) == 0);

	struct termios t;
	check("tcgetattr()", tcgetattr(s, &t) == 0 && (t.c_lflag & ICANON));
	cfmakeraw(&t);
	check("tcsetattr(): raw", tcsetattr(s, TCSANOW, &t) == 0);
	write(m, "x", 1);
	n = read(s, buf, sizeof buf);
	check("raw: a byte at a time, no echo", n == 1 && buf[0] == 'x');
	struct pollfd p = {m, POLLIN, 0};
	check("poll(): nothing for the controller", poll(&p, 1, 0) == 0);
	write(s, "y", 1);
	p.revents = 0;
	check("poll(): output for the controller", poll(&p, 1, 3000) == 1 && (p.revents & POLLIN));
	read(m, buf, sizeof buf);
	struct pollfd q = {s, POLLIN, 0};
	pid_t w = fork();
	if (w == 0) {
		usleep(200000);
		write(m, "z", 1);
		_exit(0);
	}
	check("poll(): input for the worker, from a child", poll(&q, 1, 3000) == 1);
	waitpid(w, NULL, 0);
	read(s, buf, sizeof buf);

	struct winsize ws = {.ws_row = 24, .ws_col = 80};
	struct winsize got = {0};
	check("window size", ioctl(m, TIOCSWINSZ, &ws) == 0 && ioctl(s, TIOCGWINSZ, &got) == 0 &&
	    got.ws_row == 24 && got.ws_col == 80);

	t.c_lflag |= ISIG | ICANON | ECHO;
	tcsetattr(s, TCSANOW, &t);
	int ready[2];
	pipe(ready);
	w = fork();
	if (w == 0) {
		setsid();
		ioctl(s, TIOCSCTTY, 0);
		write(ready[1], "r", 1);
		pause();
		_exit(0);
	}
	read(ready[0], buf, 1);
	usleep(100000);
	write(m, "\x03", 1);
	int status = 0;
	check("^C signals the foreground group", waitpid(w, &status, 0) == w && WIFSIGNALED(status) &&
	    WTERMSIG(status) == SIGINT);
	while (poll(&p, 1, 0) == 1 && read(m, buf, sizeof buf) > 0)
		;

	fcntl(m, F_SETFL, O_NONBLOCK);
	errno = 0;
	check("nothing to read, non-blocking: EAGAIN", read(m, buf, 1) == -1 && errno == EAGAIN);
	fcntl(m, F_SETFL, 0);
	close(s);
	usleep(100000);
	check("the worker gone: end of file at the controller", read(m, buf, 1) == 0);
	s = open(name, O_RDWR | O_NOCTTY);
	check("the worker opens again", s >= 0);
	close(m);
	usleep(100000);
	check("the controller gone: end of file at the worker", read(s, buf, 1) == 0);
	close(s);
	m = posix_openpt(O_RDWR);
	check("the pair is free again", m >= 0);
	close(m);
	printf("pty-test: %s\n", fails ? "FAILED" : "passed");
	return fails != 0;
}

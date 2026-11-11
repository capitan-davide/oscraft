/* The terminal from FreeBSD's libc on OsCraft: isatty, tcgetattr and
   tcsetattr (TIOCGETA and TIOCSETA as QNX's device commands), the window
   size, the foreground process group. */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

int
main(void)
{
	struct termios t, raw, back;
	struct winsize ws;
	int tty = isatty(0);

	printf("tty-test: isatty(0)=%d\n", tty);
	if (!tty)
		return (1);
	if (tcgetattr(0, &t) != 0) {
		printf("tty-test: tcgetattr: %s\n", strerror(errno));
		return (2);
	}
	printf("tty-test: ICANON=%d ECHO=%d ISIG=%d ICRNL=%d OPOST=%d ONLCR=%d CS8=%d\n",
	    (t.c_lflag & ICANON) != 0, (t.c_lflag & ECHO) != 0, (t.c_lflag & ISIG) != 0,
	    (t.c_iflag & ICRNL) != 0, (t.c_oflag & OPOST) != 0, (t.c_oflag & ONLCR) != 0,
	    (t.c_cflag & CSIZE) == CS8);
	printf("tty-test: VERASE=%d VEOF=%d VINTR=%d ospeed=%u\n", t.c_cc[VERASE], t.c_cc[VEOF], t.c_cc[VINTR],
	    (unsigned)cfgetospeed(&t));
	raw = t;
	cfmakeraw(&raw);
	if (tcsetattr(0, TCSANOW, &raw) != 0)
		printf("tty-test: tcsetattr(raw): %s\n", strerror(errno));
	tcgetattr(0, &back);
	printf("tty-test: raw set: ICANON=%d ECHO=%d\n", (back.c_lflag & ICANON) != 0, (back.c_lflag & ECHO) != 0);
	if (tcsetattr(0, TCSANOW, &t) != 0)
		printf("tty-test: tcsetattr(restore): %s\n", strerror(errno));
	tcgetattr(0, &back);
	printf("tty-test: restored: ICANON=%d ECHO=%d\n", (back.c_lflag & ICANON) != 0, (back.c_lflag & ECHO) != 0);
	printf("tty-test: winsize ok:%d pgrp>0:%d\n", ioctl(0, TIOCGWINSZ, &ws) == 0, tcgetpgrp(0) > 0);
	return (0);
}

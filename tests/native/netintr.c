/* A call waiting in netd ends with EINTR on a signal: recv(), accept(), on
   a TCP socket, and on an AF_UNIX pair. */
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/time.h>
#include <unistd.h>

static void
on_alarm(int s)
{
	(void)s;
}

static void
arm(void)
{
	struct itimerval t = { { 0, 0 }, { 0, 100000 } };

	setitimer(ITIMER_REAL, &t, NULL);
}

int
main(void)
{
	struct sigaction sa;
	struct sockaddr_in a;
	char b[8];
	int l, u, p[2], r;

	memset(&sa, 0, sizeof(sa));
	sa.sa_handler = on_alarm;
	sigaction(SIGALRM, &sa, NULL);

	memset(&a, 0, sizeof(a));
	a.sin_len = sizeof(a);
	a.sin_family = AF_INET;
	a.sin_port = htons(7100);
	a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	l = socket(AF_INET, SOCK_STREAM, 0);
	bind(l, (struct sockaddr *)&a, sizeof(a));
	listen(l, 1);
	arm();
	r = accept(l, NULL, NULL);
	printf("netintr: accept=%d errno=%d\n", r, r < 0 ? errno : 0);
	u = socket(AF_INET, SOCK_DGRAM, 0);
	a.sin_port = htons(7101);
	bind(u, (struct sockaddr *)&a, sizeof(a));
	arm();
	r = recv(u, b, sizeof(b), 0);
	printf("netintr: recv=%d errno=%d\n", r, r < 0 ? errno : 0);
	socketpair(AF_UNIX, SOCK_STREAM, 0, p);
	arm();
	r = read(p[0], b, sizeof(b));
	printf("netintr: unix=%d errno=%d\n", r, r < 0 ? errno : 0);
	printf("netintr: done\n");
	return (0);
}

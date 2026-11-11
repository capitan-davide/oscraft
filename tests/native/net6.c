/* IPv6 over the loopback: a UDP datagram and a TCP connection to ::1. */
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int
main(void)
{
	struct sockaddr_in6 a;
	char b[16] = { 0 };
	int s, c, l, n;

	memset(&a, 0, sizeof(a));
	a.sin6_len = sizeof(a);
	a.sin6_family = AF_INET6;
	a.sin6_addr = in6addr_loopback;
	a.sin6_port = htons(7200);
	s = socket(AF_INET6, SOCK_DGRAM, 0);
	printf("net6: udp socket=%d bind=%d\n", s >= 0, bind(s, (struct sockaddr *)&a, sizeof(a)));
	c = socket(AF_INET6, SOCK_DGRAM, 0);
	n = sendto(c, "six", 3, 0, (struct sockaddr *)&a, sizeof(a));
	printf("net6: sendto=%d errno=%d\n", n, n < 0 ? errno : 0);
	n = recv(s, b, sizeof(b), 0);
	printf("net6: udp got %.*s (%d)\n", n, b, n);
	a.sin6_port = htons(7201);
	l = socket(AF_INET6, SOCK_STREAM, 0);
	bind(l, (struct sockaddr *)&a, sizeof(a));
	listen(l, 1);
	c = socket(AF_INET6, SOCK_STREAM, 0);
	printf("net6: connect=%d\n", connect(c, (struct sockaddr *)&a, sizeof(a)));
	s = accept(l, NULL, NULL);
	write(c, "over tcp", 8);
	n = read(s, b, sizeof(b));
	printf("net6: tcp got %.*s (%d)\n", n, b, n);
	{
		/* A named AF_UNIX stream: FreeBSD's semantics, through a node. */
		struct sockaddr_un un;
		struct stat st;
		char got[8];
		int l, c, a, again;

		memset(&un, 0, sizeof(un));
		un.sun_family = AF_UNIX;
		strcpy(un.sun_path, "/tmp/net6.sock");
		un.sun_len = SUN_LEN(&un);
		unlink(un.sun_path);
		l = socket(AF_UNIX, SOCK_STREAM, 0);
		c = socket(AF_UNIX, SOCK_STREAM, 0);
		again = socket(AF_UNIX, SOCK_STREAM, 0);
		if (bind(l, (struct sockaddr *)&un, un.sun_len) != 0 || listen(l, 2) != 0)
			perror("net6: unix bind");
		if (stat(un.sun_path, &st) == 0 && S_ISSOCK(st.st_mode))
			printf("net6: unix node\n");
		if (bind(again, (struct sockaddr *)&un, un.sun_len) == -1 && errno == EADDRINUSE)
			printf("net6: unix in use\n");
		if (connect(c, (struct sockaddr *)&un, un.sun_len) != 0)
			perror("net6: unix connect");
		a = accept(l, NULL, NULL);
		if (a >= 0 && write(c, "named", 5) == 5 && read(a, got, sizeof(got)) == 5)
			printf("net6: unix got %.5s\n", got);
		close(a); close(c); close(l); close(again);
		unlink(un.sun_path);
	}
	printf("net6: done\n");
	return (0);
}

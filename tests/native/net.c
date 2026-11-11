/* FreeBSD's sockets on OsCraft, through libsys's messages to netd:
   TCP and UDP over the loopback, names, options, the stack's sysctl
   variables, the interface list and an interface request. */
#include <sys/types.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/sysctl.h>
#include <arpa/inet.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/un.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <sys/select.h>
#include <sys/event.h>
#include <errno.h>
#include <string.h>
#include <unistd.h>

static int listener;

static void *
server(void *arg)
{
	struct sockaddr_in peer;
	socklen_t len = sizeof peer;
	char buf[64];
	ssize_t n;
	int c;

	struct pollfd pf = { listener, POLLIN, 0 };
	int ready = poll(&pf, 1, 5000);
	printf("net: poll(listener)=%d revents=%#x\n", ready, pf.revents);
	c = accept(listener, (struct sockaddr *)&peer, &len);
	if (c < 0) {
		perror("accept");
		return (arg);
	}
	printf("net: accepted from %s len=%u\n", inet_ntoa(peer.sin_addr), len);
	while ((n = recv(c, buf, sizeof buf, 0)) > 0)
		send(c, buf, n, 0);
	printf("net: server saw end of stream, recv=%zd\n", n);
	close(c);
	return (arg);
}

int
main(void)
{
	struct sockaddr_in sin, bound, from;
	socklen_t len;
	pthread_t t;
	char buf[64];
	int s, u, one = 1, type;
	ssize_t n;
	int ttl;
	size_t size;
	struct ifaddrs *ifa, *p;
	struct ifreq ifr;
	void *r;

	listener = socket(AF_INET, SOCK_STREAM, 0);
	if (listener < 0) {
		perror("socket");
		return (1);
	}
	printf("net: setsockopt(SO_REUSEADDR)=%d\n", setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one));
	memset(&sin, 0, sizeof sin);
	sin.sin_len = sizeof sin;
	sin.sin_family = AF_INET;
	sin.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	sin.sin_port = 0;
	if (bind(listener, (struct sockaddr *)&sin, sizeof sin) != 0) {
		perror("bind");
		return (1);
	}
	len = sizeof bound;
	getsockname(listener, (struct sockaddr *)&bound, &len);
	printf("net: bound to %s port>0:%d len=%u\n", inet_ntoa(bound.sin_addr), ntohs(bound.sin_port) > 0, len);
	printf("net: listen=%d\n", listen(listener, 5));
	pthread_create(&t, NULL, server, NULL);

	s = socket(AF_INET, SOCK_STREAM, 0);
	len = sizeof type;
	getsockopt(s, SOL_SOCKET, SO_TYPE, &type, &len);
	printf("net: SO_TYPE=%d (SOCK_STREAM=%d)\n", type, SOCK_STREAM);
	printf("net: TCP_NODELAY=%d\n", setsockopt(s, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one));
	if (connect(s, (struct sockaddr *)&bound, sizeof bound) != 0) {
		perror("connect");
		return (1);
	}
	len = sizeof from;
	getpeername(s, (struct sockaddr *)&from, &len);
	printf("net: connected, peer port matches:%d\n", from.sin_port == bound.sin_port);
	n = send(s, "hello, stack", 12, 0);
	printf("net: send=%zd\n", n);
	n = recv(s, buf, sizeof buf, 0);
	printf("net: echo=%.*s (%zd)\n", (int)n, buf, n);
	{
		struct iovec iov[2] = { { "ab", 2 }, { "cd", 2 } };
		struct msghdr m;
		memset(&m, 0, sizeof m);
		m.msg_iov = iov;
		m.msg_iovlen = 2;
		printf("net: sendmsg=%zd", sendmsg(s, &m, 0));
		iov[0].iov_base = buf;
		iov[0].iov_len = 3;
		iov[1].iov_base = buf + 3;
		iov[1].iov_len = 1;
		m.msg_iovlen = 2;
		n = recvmsg(s, &m, MSG_WAITALL);
		printf(" recvmsg=%zd %.*s flags=%#x\n", n, (int)n, buf, m.msg_flags);
	}
	printf("net: shutdown=%d\n", shutdown(s, SHUT_WR));
	pthread_join(t, &r);
	close(s);

	{
		/*
		 * A connected datagram socket, as syslogd's forwarding has:
		 * bound to any address and no port, connect(), then send()
		 * with no address.
		 */
		struct sockaddr_in to = sin, any;
		struct pollfd pf;
		char b[16] = { 0 };
		ssize_t got = -1, sent;
		int r = socket(AF_INET, SOCK_DGRAM, 0), c = socket(AF_INET, SOCK_DGRAM | SOCK_CLOEXEC, 0);
		int bc, cc, e;

		to.sin_port = 0;
		bind(r, (struct sockaddr *)&to, sizeof to);
		len = sizeof to;
		getsockname(r, (struct sockaddr *)&to, &len);
		/* sin_len left zero, as Rust's standard library leaves it:
		   the call's length counts (the kernel's getsockaddr()). */
		memset(&any, 0, sizeof any);
		any.sin_family = AF_INET;
		to.sin_len = 0;
		bc = bind(c, (struct sockaddr *)&any, sizeof any);
		errno = 0;
		cc = connect(c, (struct sockaddr *)&to, sizeof to);
		sent = send(c, "connected", 9, MSG_NOSIGNAL);
		e = errno;
		pf.fd = r;
		pf.events = POLLIN;
		if (poll(&pf, 1, 2000) == 1)
			got = recv(r, b, sizeof b - 1, 0);
		printf("net: connected udp: bind=%d connect=%d send=%zd errno=%d recv=%zd %s\n", bc, cc, sent, e, got, b);
		close(r);
		close(c);
	}

	u = socket(AF_INET, SOCK_DGRAM, 0);
	sin.sin_port = 0;
	bind(u, (struct sockaddr *)&sin, sizeof sin);
	len = sizeof bound;
	getsockname(u, (struct sockaddr *)&bound, &len);
	{
		struct pollfd pf = { u, POLLIN, 0 };
		fd_set rs;
		struct timeval tv = { 0, 50000 };
		printf("net: poll(idle udp, 50ms)=%d\n", poll(&pf, 1, 50));
		FD_ZERO(&rs);
		FD_SET(u, &rs);
		printf("net: select(idle udp)=%d\n", select(u + 1, &rs, NULL, NULL, &tv));
		n = sendto(u, "datagram", 8, 0, (struct sockaddr *)&bound, sizeof bound);
		printf("net: poll(udp after sendto)=%d revents=%#x\n", poll(&pf, 1, 2000), pf.revents);
		FD_ZERO(&rs);
		FD_SET(u, &rs);
		printf("net: select(udp)=%d set:%d\n", select(u + 1, &rs, NULL, NULL, NULL), FD_ISSET(u, &rs));
	}
	{
		int p[2];
		struct pollfd pf;
		fd_set rs, ws;
		pipe(p);
		pf.fd = p[0];
		pf.events = POLLIN;
		printf("net: poll(empty pipe)=%d", poll(&pf, 1, 0));
		write(p[1], "x", 1);
		printf(" after write=%d revents=%#x", poll(&pf, 1, 1000), pf.revents);
		FD_ZERO(&rs);
		FD_SET(p[0], &rs);
		FD_ZERO(&ws);
		FD_SET(p[1], &ws);
		printf(" select=%d r:%d w:%d\n", select(p[1] + 1, &rs, &ws, NULL, NULL), FD_ISSET(p[0], &rs), FD_ISSET(p[1], &ws));
		close(p[0]);
		close(p[1]);
	}
	{
		int kq = kqueue();
		struct kevent ch[3], ev[4];
		struct timespec to = { 0, 50000000 };
		struct sockaddr_in me;
		int k, u2 = socket(AF_INET, SOCK_DGRAM, 0);
		len = sizeof me;
		bind(u2, (struct sockaddr *)&sin, sizeof sin);
		getsockname(u2, (struct sockaddr *)&me, &len);
		EV_SET(&ch[0], u2, EVFILT_READ, EV_ADD, 0, 0, (void *)1);
		EV_SET(&ch[1], 1, EVFILT_TIMER, EV_ADD | EV_ONESHOT, 0, 20, (void *)2);
		EV_SET(&ch[2], 7, EVFILT_VNODE, EV_ADD, 0, 0, (void *)3);
		k = kevent(kq, ch, 3, ev, 4, &to);
		printf("net: kqueue=%d kevent(add 3, 50ms)=%d", kq >= 0, k);
		printf(" first: vnode error=%d(EINVAL=%d)", (ev[0].flags & EV_ERROR) != 0 && ev[0].filter == EVFILT_VNODE ? (int)ev[0].data : -1, EINVAL);
		printf(" second: timer=%d data=%d", k > 1 && ev[1].filter == EVFILT_TIMER, k > 1 ? (int)ev[1].data : -1);
		sendto(u2, "datagram", 8, 0, (struct sockaddr *)&me, sizeof me);
		k = kevent(kq, NULL, 0, ev, 4, &to);
		printf(" after sendto: %d read data=%d udata=%d", k, k > 0 ? (int)ev[0].data : -1, k > 0 ? (int)(long)ev[0].udata : -1);
		EV_SET(&ch[0], 9, EVFILT_USER, EV_ADD | EV_CLEAR, 0, 0, (void *)4);
		kevent(kq, ch, 1, NULL, 0, NULL);
		EV_SET(&ch[0], 9, EVFILT_USER, 0, NOTE_TRIGGER, 0, NULL);
		k = kevent(kq, ch, 1, ev, 4, &to);
		printf(" user: %d", k);
		for (int i = 0; i < k; i++)
			if (ev[i].filter == EVFILT_USER)
				printf(" triggered udata=%d", (int)(long)ev[i].udata);
		printf("\n");
		close(u2);
		close(kq);
	}
	{
		/* SCM_RIGHTS over a socketpair: a pipe's read end crosses as a
		   dupfd handle; AF_UNIX names need a file system with socket
		   nodes, which /dev/shmem is not. */
		int sp[2], p[2], got = -1;
		struct msghdr m;
		struct iovec iov;
		union { struct cmsghdr h; char b[CMSG_SPACE(sizeof(int))]; } c;
		struct cmsghdr *cm;
		struct sockaddr_un un;
		char ch = '?';
		printf("net: socketpair=%d", socketpair(AF_UNIX, SOCK_STREAM, 0, sp));
		pipe(p);
		memset(&m, 0, sizeof m);
		iov.iov_base = "x";
		iov.iov_len = 1;
		m.msg_iov = &iov;
		m.msg_iovlen = 1;
		m.msg_control = c.b;
		m.msg_controllen = sizeof c.b;
		cm = CMSG_FIRSTHDR(&m);
		cm->cmsg_level = SOL_SOCKET;
		cm->cmsg_type = SCM_RIGHTS;
		cm->cmsg_len = CMSG_LEN(sizeof(int));
		memcpy(CMSG_DATA(cm), &p[0], sizeof(int));
		printf(" sendmsg(rights)=%zd", sendmsg(sp[0], &m, 0));
		memset(c.b, 0, sizeof c.b);
		iov.iov_base = &ch;
		m.msg_controllen = sizeof c.b;
		n = recvmsg(sp[1], &m, 0);
		cm = CMSG_FIRSTHDR(&m);
		if (n == 1 && cm != NULL && cm->cmsg_type == SCM_RIGHTS)
			memcpy(&got, CMSG_DATA(cm), sizeof(int));
		printf(" recvmsg=%zd data=%c fd received:%d differs:%d", n, ch, got >= 0, got != p[0]);
		write(p[1], "through", 7);
		n = read(got, buf, sizeof buf);
		printf(" read via it=%.*s\n", (int)n, buf);
		close(got);
		close(p[0]);
		close(p[1]);
		close(sp[0]);
		close(sp[1]);
		memset(&un, 0, sizeof un);
		un.sun_family = AF_UNIX;
		strcpy(un.sun_path, "/tmp/net-sock");
		un.sun_len = SUN_LEN(&un);
		unlink(un.sun_path);
		s = socket(AF_UNIX, SOCK_STREAM, 0);
		got = bind(s, (struct sockaddr *)&un, sizeof un);
		printf("net: unix bind on shmem=%d (errno %d)\n", got, got ? errno : 0);
		close(s);
		unlink(un.sun_path);
	}
	n = 0;
	len = sizeof from;
	n = recvfrom(u, buf, sizeof buf, 0, (struct sockaddr *)&from, &len);
	printf("net: udp %.*s (%zd) from port matches:%d len=%u\n", (int)n, buf, n, from.sin_port == bound.sin_port, len);
	close(u);

	size = sizeof ttl;
	printf("net: sysctlbyname(net.inet.ip.ttl)=%d ttl=%d\n", sysctlbyname("net.inet.ip.ttl", &ttl, &size, NULL, 0), ttl);
	if (getifaddrs(&ifa) == 0) {
		for (p = ifa; p != NULL; p = p->ifa_next)
			if (p->ifa_addr != NULL && p->ifa_addr->sa_family == AF_INET)
				printf("net: ifaddr %s %s\n", p->ifa_name, inet_ntoa(((struct sockaddr_in *)p->ifa_addr)->sin_addr));
		freeifaddrs(ifa);
	} else
		perror("getifaddrs");
	memset(&ifr, 0, sizeof ifr);
	strcpy(ifr.ifr_name, "lo0");
	u = socket(AF_INET, SOCK_DGRAM, 0);
	printf("net: SIOCGIFFLAGS=%d loopback:%d up:%d\n", ioctl(u, SIOCGIFFLAGS, &ifr), (ifr.ifr_flags & IFF_LOOPBACK) != 0, (ifr.ifr_flags & IFF_UP) != 0);
	close(u);
	close(listener);
	printf("net: done\n");
	return (0);
}

/*
 * netd's first real interface: vtnet0, FreeBSD's virtio network driver on
 * QEMU's virtio device (pci.ko, virtio.ko, virtio_pci.ko, if_vtnet.ko,
 * loaded by the boot script), on QEMU's user-mode network: the guest is
 * 10.0.2.15, and 10.0.2.2 is the gateway and the host itself.
 *
 * - the interface as getifaddrs() gives it: its link address (QEMU's
 *   default, 52:54:00:12:34:56), its IPv4 address, up and running;
 * - TCP through the device and out of QEMU: a connect() to a port of the
 *   host nothing listens on (tcpmux, 1) is refused by the host, not by
 *   this system: the SYN left through vtnet0 and the RST came back (a
 *   system without the route fails with ENETUNREACH, a dead device with
 *   ETIMEDOUT).
 */
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <arpa/inet.h>
#include <net/if.h>
#include <net/if_dl.h>
#include <netinet/in.h>
#include <errno.h>
#include <ifaddrs.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

int
main(void)
{
	struct ifaddrs *ifa, *p;
	struct sockaddr_in sin;
	struct timeval tv = { 20, 0 };
	int fails = 0, link = 0, inet = 0, up = 0, s, r, e;

	if (getifaddrs(&ifa) != 0) {
		printf("vtnet: getifaddrs: errno %d\n", errno);
		return (1);
	}
	for (p = ifa; p != NULL; p = p->ifa_next) {
		if (strcmp(p->ifa_name, "vtnet0") != 0 || p->ifa_addr == NULL)
			continue;
		if (p->ifa_addr->sa_family == AF_LINK) {
			struct sockaddr_dl *dl = (struct sockaddr_dl *)p->ifa_addr;
			unsigned char *m = (unsigned char *)LLADDR(dl);

			link = dl->sdl_alen == 6;
			up = (p->ifa_flags & (IFF_UP | IFF_RUNNING)) ==
			    (IFF_UP | IFF_RUNNING);
			printf("vtnet: ether %02x:%02x:%02x:%02x:%02x:%02x up=%d\n",
			    m[0], m[1], m[2], m[3], m[4], m[5], up);
		} else if (p->ifa_addr->sa_family == AF_INET) {
			char b[INET_ADDRSTRLEN];

			inet = 1;
			printf("vtnet: inet %s\n", inet_ntop(AF_INET,
			    &((struct sockaddr_in *)p->ifa_addr)->sin_addr, b,
			    sizeof(b)));
		}
	}
	freeifaddrs(ifa);
	if (!link || !inet || !up) {
		printf("vtnet: FAILED: vtnet0 link=%d inet=%d up=%d\n", link,
		    inet, up);
		fails++;
	}

	s = socket(AF_INET, SOCK_STREAM, 0);
	memset(&sin, 0, sizeof(sin));
	sin.sin_len = sizeof(sin);
	sin.sin_family = AF_INET;
	sin.sin_port = htons(1);
	sin.sin_addr.s_addr = inet_addr("10.0.2.2");
	(void)setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv));
	r = connect(s, (struct sockaddr *)&sin, sizeof(sin));
	e = errno;
	printf("vtnet: connect to the host's port 1: %d errno %d (%s)\n", r,
	    r == 0 ? 0 : e, r == 0 ? "connected" : strerror(e));
	if (r == 0 || e != ECONNREFUSED) {
		printf("vtnet: FAILED: expected ECONNREFUSED (%d)\n",
		    ECONNREFUSED);
		fails++;
	}
	close(s);
	printf(fails == 0 ? "vtnet: passed\n" : "vtnet: %d FAILED\n", fails);
	return (fails != 0);
}

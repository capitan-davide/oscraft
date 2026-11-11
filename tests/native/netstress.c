/* Load on netd: threads moving a lot of data over TCP at once, checked byte
   by byte, then many connections made and closed, and UDP in bursts. */
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define PAIRS	4
#define MB	(8)
#define CHUNK	8192
#define CYCLES	400

static int bad;

static void
address(struct sockaddr_in *a, int port)
{
	memset(a, 0, sizeof(*a));
	a->sin_len = sizeof(*a);
	a->sin_family = AF_INET;
	a->sin_port = htons(port);
	a->sin_addr.s_addr = htonl(INADDR_LOOPBACK);
}

static int
listener(int port)
{
	struct sockaddr_in a;
	int one = 1, l = socket(AF_INET, SOCK_STREAM, 0);

	setsockopt(l, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
	address(&a, port);
	if (bind(l, (struct sockaddr *)&a, sizeof(a)) != 0 || listen(l, 64) != 0)
		perror("listener");
	return (l);
}

static void *
receiver(void *arg)
{
	int l = listener((int)(long)arg), s = accept(l, NULL, NULL);
	unsigned char b[CHUNK];
	unsigned long at = 0;
	ssize_t n;

	while ((n = read(s, b, sizeof(b))) > 0) {
		for (ssize_t i = 0; i < n; i++)
			if (b[i] != (unsigned char)(at + i))
				__sync_fetch_and_add(&bad, 1);
		at += n;
	}
	if (at != MB * 1024UL * 1024)
		__sync_fetch_and_add(&bad, 1);
	close(s);
	close(l);
	return (NULL);
}

static void *
sender(void *arg)
{
	struct sockaddr_in a;
	unsigned char b[CHUNK];
	unsigned long at = 0, end = MB * 1024UL * 1024;
	int s = socket(AF_INET, SOCK_STREAM, 0), tries = 0;

	address(&a, (int)(long)arg);
	while (connect(s, (struct sockaddr *)&a, sizeof(a)) != 0 && tries++ < 100) {
		close(s);
		s = socket(AF_INET, SOCK_STREAM, 0);
		usleep(10000);
	}
	while (at < end) {
		size_t n = end - at < CHUNK ? end - at : CHUNK;

		for (size_t i = 0; i < n; i++)
			b[i] = (unsigned char)(at + i);
		ssize_t w = write(s, b, n);
		if (w <= 0) {
			__sync_fetch_and_add(&bad, 1);
			break;
		}
		at += w;
	}
	close(s);
	return (NULL);
}

static double
now(void)
{
	struct timespec t;

	clock_gettime(CLOCK_MONOTONIC, &t);
	return (t.tv_sec + t.tv_nsec / 1e9);
}

int
main(void)
{
	pthread_t t[PAIRS * 2];
	struct sockaddr_in a;
	char b[64];
	double t0 = now();
	int i;

	for (i = 0; i < PAIRS; i++) {
		pthread_create(&t[i * 2], NULL, receiver, (void *)(long)(7400 + i));
		pthread_create(&t[i * 2 + 1], NULL, sender, (void *)(long)(7400 + i));
	}
	for (i = 0; i < PAIRS * 2; i++)
		pthread_join(t[i], NULL);
	printf("netstress: %d MB over %d connections, %d bad, %.1f s\n", MB * PAIRS, PAIRS, bad, now() - t0);

	int l = listener(7500);
	for (i = 0; i < CYCLES; i++) {
		int c = socket(AF_INET, SOCK_STREAM, 0), s;

		address(&a, 7500);
		if (connect(c, (struct sockaddr *)&a, sizeof(a)) != 0) {
			printf("netstress: connect %d: errno %d\n", i, errno);
			bad++;
			break;
		}
		s = accept(l, NULL, NULL);
		if (write(c, "x", 1) != 1 || read(s, b, 1) != 1)
			bad++;
		close(c);
		close(s);
	}
	printf("netstress: %d connections, %d bad\n", i, bad);

	int u = socket(AF_INET, SOCK_DGRAM, 0), v = socket(AF_INET, SOCK_DGRAM, 0), got = 0;
	address(&a, 7501);
	bind(u, (struct sockaddr *)&a, sizeof(a));
	for (i = 0; i < 2000; i++) {
		sendto(v, "ping", 4, 0, (struct sockaddr *)&a, sizeof(a));
		if (recv(u, b, sizeof(b), 0) == 4)
			got++;
	}
	printf("netstress: %d datagrams\n", got);
	printf("netstress: %s\n", bad == 0 && got == 2000 ? "passed" : "FAILED");
	return (0);
}

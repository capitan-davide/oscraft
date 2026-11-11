/* Pipes on OsCraft's native pipe manager (user/pipe, over the kernel's
   pipes), the checks of tests/qnx/kpipetest.c as a FreeBSD program:
   - a pipe holds 64 KB; non-blocking writes take what fits (EAGAIN if
     nothing), but up to PIPE_BUF (512) all or nothing;
   - an empty pipe: EAGAIN if non-blocking; end of file once the writers
     are gone; a write without readers is SIGPIPE and EPIPE; no lseek()
     (ESPIPE);
   - a blocked reader is woken by a write, a blocked writer by a read;
   - poll() hears of data and of room;
   - dup()ed and inherited descriptors are the same pipe;
   - data goes through in order, across processes;
   - a read() into a buffer that faults is EFAULT and leaves the data in
     the pipe.
   Prints one "ok"/"FAIL" line per check and "pipe-test: passed". */
#include <sys/filio.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int fails;
static volatile int sigpipes;

static void
check(const char *what, int ok)
{
	printf("pipe-test: %s %s\n", ok ? "ok" : "FAIL", what);
	fflush(stdout);
	fails += !ok;
}

static void
on_sigpipe(int sig)
{
	(void)sig;
	sigpipes++;
}

int
main(void)
{
	static char buf[200000];
	memset(buf, 'k', sizeof buf);

	int p[2];
	check("pipe()", pipe(p) == 0);
	struct stat st;
	fstat(p[0], &st);
	check("fstat(): a FIFO, mode 0777", S_ISFIFO(st.st_mode) && (st.st_mode & 0777) == 0777);

	fcntl(p[1], F_SETFL, O_NONBLOCK);
	int total = 0, n;
	while ((n = write(p[1], buf, 4096)) > 0)
		total += n;
	check("it holds 64 KB, then EAGAIN", total == 65536 && errno == EAGAIN);
	read(p[0], buf, 300);
	errno = 0;
	check("an atomic write (400) into 300 free: EAGAIN",
	    write(p[1], buf, 400) == -1 && errno == EAGAIN);
	check("a larger write (10000) takes the 300 that fit", write(p[1], buf, 10000) == 300);
	struct pollfd pf = {p[1], POLLOUT, 0};
	check("poll(): a full pipe isn't writable", poll(&pf, 1, 0) == 0);
	read(p[0], buf, 6000);
	check("6000 read: writable again", poll(&pf, 1, 0) == 1);
	fcntl(p[0], F_SETFL, O_NONBLOCK);
	while (read(p[0], buf, sizeof buf) > 0)
		;
	errno = 0;
	check("an empty pipe, non-blocking: EAGAIN", read(p[0], buf, 10) == -1 && errno == EAGAIN);
	errno = 0;
	check("lseek(): ESPIPE", lseek(p[0], 0, SEEK_SET) == -1 && errno == ESPIPE);
	close(p[1]);
	usleep(200000);
	check("the writer gone: end of file", read(p[0], buf, 10) == 0);
	close(p[0]);

	signal(SIGPIPE, on_sigpipe);
	pipe(p);
	close(p[0]);
	usleep(200000);
	errno = 0;
	check("a write without readers: EPIPE", write(p[1], "x", 1) == -1 && errno == EPIPE);
	check("... and SIGPIPE", sigpipes == 1);
	close(p[1]);

	/* A reader blocked until a child writes; the child blocked until the
	 * parent reads. */
	pipe(p);
	pid_t pid = fork();
	if (pid == 0) {
		close(p[0]);
		usleep(200000);
		if (write(p[1], "hello", 5) != 5)
			printf("pipe-test:   child's write: errno %d\n", errno);
		int w = 0;
		while (w < 65536 + 100) {
			int k = write(p[1], buf, 4096);
			if (k <= 0)
				break;
			w += k;
		}
		_exit(w >= 65536 + 100 ? 0 : 1);
	}
	close(p[1]);
	char got[8] = "";
	check("a blocked read woken by the child's write",
	    read(p[0], got, 5) == 5 && !strcmp(got, "hello"));
	usleep(300000);
	long all = 0;
	while ((n = read(p[0], buf, sizeof buf)) > 0)
		all += n;
	int status = -1;
	waitpid(pid, &status, 0);
	check("the child's blocked write went on as the pipe emptied",
	    WIFEXITED(status) && WEXITSTATUS(status) == 0 && all >= 65536);
	close(p[0]);

	/* poll() for input across processes, a dup()ed end, an inherited one. */
	pipe(p);
	int d = dup(p[1]);
	if (d < 0)
		printf("pipe-test:   dup: errno %d\n", errno);
	pid = fork();
	if (pid == 0) {
		close(p[0]);
		close(p[1]);
		usleep(200000);
		if (write(d, "dup", 3) != 3)
			printf("pipe-test:   child's write: errno %d\n", errno);
		_exit(0);
	}
	close(d);
	close(p[1]);
	struct pollfd in = {p[0], POLLIN, 0};
	int r = poll(&in, 1, 3000);
	check("poll() wakes when the child writes (through a dup)", r == 1 && (in.revents & POLLIN));
	memset(got, 0, sizeof got);
	errno = 0;
	n = read(p[0], got, 3);
	if (n != 3)
		printf("pipe-test:   read %d errno %d revents %#x\n", n, errno, in.revents);
	check("the data", n == 3 && !strcmp(got, "dup"));
	waitpid(pid, NULL, 0);
	in.revents = 0;
	r = poll(&in, 1, 3000);
	check("poll() hears of the last writer's end", r == 1 && (in.revents & (POLLIN | POLLHUP)));
	check("then end of file", read(p[0], got, 1) == 0);
	close(p[0]);

	/* 5 MB through, in order. */
	pipe(p);
	pid = fork();
	if (pid == 0) {
		close(p[0]);
		for (int i = 0; i < 1000; i++) {
			memset(buf, 'a' + i % 26, 5000);
			write(p[1], buf, 5000);
		}
		_exit(0);
	}
	close(p[1]);
	long sum = 0, bad = 0;
	static char in5[5000];
	for (int i = 0; i < 1000; i++) {
		int k = 0;
		while (k < 5000 && (n = read(p[0], in5 + k, 5000 - k)) > 0)
			k += n;
		sum += k;
		bad += k != 5000 || in5[0] != 'a' + i % 26 || in5[4999] != 'a' + i % 26;
	}
	waitpid(pid, NULL, 0);
	check("5 MB in 5000-byte writes, in order", sum == 5000000 && bad == 0);
	close(p[0]);

	pipe(p);
	fcntl(p[0], F_SETFL, O_NONBLOCK);
	static char data[6000];
	for (int i = 0; i < 6000; i++)
		data[i] = 'a' + i % 26;
	char *area = mmap(NULL, 8192, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
	mprotect(area, 8192, PROT_NONE);
	write(p[1], data, 6000);
	errno = 0;
	r = read(p[0], area, 100);
	check("a read() into an unmapped buffer: EFAULT", r == -1 && errno == EFAULT);
	mprotect(area, 4096, PROT_READ | PROT_WRITE);
	errno = 0;
	r = read(p[0], area, 8192);
	check("a read() into a half-mapped one: EFAULT", r == -1 && errno == EFAULT);
	static char back[8000];
	r = read(p[0], back, sizeof back);
	check("the data stays in the pipe, all of it, in order", r == 6000 && memcmp(back, data, 6000) == 0);
	close(p[0]);
	close(p[1]);
	munmap(area, 8192);

	/* poll() as sys_pipe.c's pipe_poll(): POLLHUP with the other end gone,
	   asked for or not, POLLIN or POLLOUT still as they apply. */
	pipe(p);
	pf = (struct pollfd){ p[0], POLLIN, 0 };
	check("poll: empty, a writer there", poll(&pf, 1, 0) == 0 && pf.revents == 0);
	write(p[1], "x", 1);
	check("poll: data", poll(&pf, 1, 0) == 1 && pf.revents == POLLIN);
	close(p[1]);
	check("poll: data, the writer gone", poll(&pf, 1, 0) == 1 && pf.revents == (POLLIN | POLLHUP));
	read(p[0], &pf.revents, 1);
	check("poll: empty, the writer gone", poll(&pf, 1, 0) == 1 && pf.revents == (POLLIN | POLLHUP));
	pf.events = 0;
	check("poll: POLLHUP needs no asking", poll(&pf, 1, 0) == 1 && pf.revents == POLLHUP);
	close(p[0]);
	pipe(p);
	close(p[0]);
	struct pollfd pw = { p[1], POLLOUT, 0 };
	check("poll: the reader gone", poll(&pw, 1, 0) == 1 && pw.revents == (POLLOUT | POLLHUP));
	close(p[1]);

	/* A hangup wakes a poll() that is waiting. */
	pipe(p);
	pid_t waker = fork();
	if (waker == 0) {
		close(p[0]);
		usleep(100000);
		_exit(0);
	}
	close(p[1]);
	struct pollfd pi = { p[0], POLLIN, 0 };
	check("poll: woken by the hangup", poll(&pi, 1, 5000) == 1 && pi.revents == (POLLIN | POLLHUP));
	waitpid(waker, NULL, 0);
	close(p[0]);

	/* What is waiting, and PIPE_BUF. */
	pipe(p);
	write(p[1], "hello", 5);
	int waiting = -1;
	check("FIONREAD: what can be read", ioctl(p[0], FIONREAD, &waiting) == 0 && waiting == 5);
	fstat(p[0], &st);
	check("st_size: the same", st.st_size == 5);
	check("fpathconf(_PC_PIPE_BUF): 512", fpathconf(p[0], _PC_PIPE_BUF) == 512);
	close(p[0]);
	close(p[1]);

	printf("pipe-test: %s\n", fails ? "FAILED" : "passed");
	return fails != 0;
}

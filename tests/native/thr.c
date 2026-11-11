/* FreeBSD's libthr on OsCraft: threads, a mutex, a condition variable,
   a join, thread IDs. */
#include <pthread.h>
#include <pthread_np.h>
#include <sched.h>
#include <sys/cpuset.h>
#include <stdio.h>
#include <stdlib.h>
#include <errno.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>

#define THREADS 4
#define ROUNDS 20000

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_rwlock_t rw = PTHREAD_RWLOCK_INITIALIZER;
static long rw_shared, rw_max_readers, rw_readers;
static pthread_cond_t ready = PTHREAD_COND_INITIALIZER;
static long counter;
static int handshakes;

static void *
count(void *arg)
{
	int i;

	for (i = 0; i < ROUNDS; i++) {
		pthread_mutex_lock(&lock);
		counter++;
		pthread_mutex_unlock(&lock);
	}
	return (arg);
}

/* Readers count how many hold the lock at once; writers bump the
   shared value under the write lock, which no reader may see half done. */
static void *
reader(void *arg)
{
	int i;

	for (i = 0; i < 2000; i++) {
		pthread_rwlock_rdlock(&rw);
		pthread_mutex_lock(&lock);
		if (++rw_readers > rw_max_readers)
			rw_max_readers = rw_readers;
		pthread_mutex_unlock(&lock);
		if (rw_shared % 2 != 0)
			rw_shared = -1; /* a torn update: never */
		pthread_mutex_lock(&lock);
		rw_readers--;
		pthread_mutex_unlock(&lock);
		pthread_rwlock_unlock(&rw);
	}
	return (arg);
}

static void *
writer(void *arg)
{
	int i;

	for (i = 0; i < 1000; i++) {
		pthread_rwlock_wrlock(&rw);
		rw_shared++;
		rw_shared++;
		pthread_rwlock_unlock(&rw);
	}
	return (arg);
}

/* libthr's lock around a static rwlock's first use (hidden, so only the
   static link sees it). */
extern struct { unsigned owner, flags; } _rwlock_static_lock __attribute__((weak));

/* A watchdog: the lock's raw words (libthr's struct urwlock is the
   first member of its rwlock) if the test is still running after 3 s. */
static void *
watchdog(void *arg)
{
	unsigned *w;

	sleep(3);
	w = (unsigned *)rw;
	if (w == NULL)
		printf("thr-test: watchdog: the rwlock was never initialised\n");
	else
		printf("thr-test: watchdog: state=%#x flags=%#x blocked_readers=%u blocked_writers=%u shared=%ld\n", w[0], w[1], w[2], w[3], rw_shared);
	if (&_rwlock_static_lock != NULL)
		printf("thr-test: watchdog: static lock owner=%#x flags=%#x\n", _rwlock_static_lock.owner, _rwlock_static_lock.flags);
	fflush(stdout);
	system("/compat/qnx/bin/pidin -p thr-test -F '%a %b %N %p %J %B %K'");
	_exit(2);
}

static void *
consumer(void *arg)
{
	int n = 0;

	pthread_mutex_lock(&lock);
	while (n < 3) {
		while (handshakes == n)
			pthread_cond_wait(&ready, &lock);
		n = handshakes;
	}
	pthread_mutex_unlock(&lock);
	return (arg);
}

static pthread_mutex_t pi;

static void *
high(void *arg)
{
	int r = pthread_mutex_lock(&pi);

	printf("thr-test: high: lock=%d unlock=%d\n", r, pthread_mutex_unlock(&pi));
	return (arg);
}

/* "p": a priority-inheriting mutex held by the main thread (priority 10),
   wanted by a SCHED_FIFO thread at priority 5 (16 on OsCraft): pidin
   shows the holder raised to 16 while the thread waits, then back.
   system() blocks SIGCHLD meanwhile: the pending signal must not end the
   waiter's wait. */
static int
inherit(void)
{
	pthread_mutexattr_t ma;
	pthread_attr_t ta;
	struct sched_param sp = { .sched_priority = 5 };
	pthread_t t;
	void *r;
	int policy;

	pthread_mutexattr_init(&ma);
	pthread_mutexattr_setprotocol(&ma, PTHREAD_PRIO_INHERIT);
	pthread_mutex_init(&pi, &ma);
	pthread_attr_init(&ta);
	pthread_attr_setinheritsched(&ta, PTHREAD_EXPLICIT_SCHED);
	pthread_attr_setschedpolicy(&ta, SCHED_FIFO);
	pthread_attr_setschedparam(&ta, &sp);
	pthread_mutex_lock(&pi);
	if (pthread_create(&t, &ta, high, NULL) != 0) {
		printf("thr-test: pthread_create failed\n");
		return (2);
	}
	usleep(200000);
	fflush(stdout);
	system("/compat/qnx/bin/pidin -p thr-test");
	policy = -1;
	printf("thr-test: waiter getschedparam=%d policy=%d priority=%d\n", pthread_getschedparam(t, &policy, &sp), policy, sp.sched_priority);
	pthread_getschedparam(pthread_self(), &policy, &sp);
	printf("thr-test: holder policy=%d priority=%d\n", policy, sp.sched_priority);
	pthread_mutex_unlock(&pi);
	pthread_join(t, &r);
	fflush(stdout);
	system("/compat/qnx/bin/pidin -p thr-test");
	return (0);
}

static void *
named(void *arg)
{
	pthread_set_name_np(pthread_self(), "helper-thread-with-a-long-name");
	pthread_mutex_lock(&pi);
	pthread_mutex_unlock(&pi);
	return (arg);
}

/* "n": thread names (thr_set_name, cut to MAXCOMLEN) and CPU affinity
   (cpuset_setaffinity, the thread's runmask) shown by pidin. */
static int
names(void)
{
	cpuset_t set;
	pthread_t t;
	void *r;
	int n, i;

	pthread_mutex_lock(&pi);
	pthread_set_name_np(pthread_self(), "main");
	pthread_create(&t, NULL, named, NULL);
	usleep(100000);
	CPU_ZERO(&set);
	CPU_SET(1, &set);
	printf("thr-test: setaffinity(cpu 1)=%d\n", pthread_setaffinity_np(pthread_self(), sizeof set, &set));
	CPU_ZERO(&set);
	printf("thr-test: getaffinity=%d", pthread_getaffinity_np(pthread_self(), sizeof set, &set));
	for (n = i = 0; i < CPU_SETSIZE; i++)
		if (CPU_ISSET(i, &set)) {
			printf(" cpu%d", i);
			n++;
		}
	printf(" (%d set)\n", n);
	printf("thr-test: setaffinity(other thread)=%d\n", pthread_setaffinity_np(t, sizeof set, &set));
	fflush(stdout);
	system("/compat/qnx/bin/pidin -p thr-test -F '%b %h %i'");
	pthread_mutex_unlock(&pi);
	pthread_join(t, &r);
	return (0);
}

/* "s": the sigwait family over SignalWaitinfo: a blocked SIGUSR1 raised
   and taken with its sender; a limit that passes (EAGAIN); sigwait. */
static int
sigwaits(void)
{
	sigset_t set;
	siginfo_t info;
	struct timespec ts = { 0, 10000000 };
	union sigval v;
	int sig = 0, r;

	sigemptyset(&set);
	sigaddset(&set, SIGUSR1);
	sigaddset(&set, SIGUSR2);
	pthread_sigmask(SIG_BLOCK, &set, NULL);
	v.sival_int = 42;
	sigqueue(getpid(), SIGUSR1, v);
	memset(&info, 0, sizeof info);
	r = sigwaitinfo(&set, &info);
	printf("thr-test: sigwaitinfo=%d (SIGUSR1=%d) si_pid matches:%d si_value=%d\n", r, SIGUSR1, info.si_pid == getpid(), info.si_value.sival_int);
	errno = 0;
	r = sigtimedwait(&set, &info, &ts);
	printf("thr-test: sigtimedwait(10ms, none)=%d errno=%d (EAGAIN=%d)\n", r, errno, EAGAIN);
	kill(getpid(), SIGUSR2);
	r = sigwait(&set, &sig);
	printf("thr-test: sigwait=%d sig=%d (SIGUSR2=%d)\n", r, sig, SIGUSR2);
	return (0);
}

int
main(int argc, char *argv[])
{
	pthread_t t[THREADS], c;
	void *r;
	int i, readers = 2, writers = 2;

	/* "r": readers only; "w": writers only; "rw" or nothing: both. */
	if (argc > 1 && argv[1][0] == 'p')
		return (inherit());
	if (argc > 1 && argv[1][0] == 'n')
		return (names());
	if (argc > 1 && argv[1][0] == 's')
		return (sigwaits());
	if (argc > 1) {
		readers = strchr(argv[1], 'r') != NULL ? 2 : 0;
		writers = strchr(argv[1], 'w') != NULL ? 2 : 0;
		if (argv[1][0] == '1') {
			readers = 1;
			writers = 1;
		}
	}

	printf("thr-test: main thread, self=%p\n", (void *)pthread_self());
	if (argc > 1 && strchr(argv[1], 'q') != NULL)
		goto rwlock; /* quick: the rwlock phase only */
	for (i = 0; i < THREADS; i++)
		if (pthread_create(&t[i], NULL, count, (void *)(long)(i + 1)) != 0) {
			printf("thr-test: pthread_create failed\n");
			return (2);
		}
	for (i = 0; i < THREADS; i++) {
		pthread_join(t[i], &r);
		printf("thr-test: joined thread %d, returned %ld\n", i, (long)r);
	}
	printf("thr-test: counter=%ld (expected %d)\n", counter, THREADS * ROUNDS);
	pthread_create(&c, NULL, consumer, (void *)7L);
	for (i = 0; i < 3; i++) {
		pthread_mutex_lock(&lock);
		handshakes++;
		pthread_cond_signal(&ready);
		pthread_mutex_unlock(&lock);
	}
	pthread_join(c, &r);
	printf("thr-test: consumer saw %d handshakes, returned %ld\n", handshakes, (long)r);
rwlock:
	if (argc <= 1 || strchr(argv[1], 'x') == NULL) {
		pthread_rwlock_rdlock(&rw);
		pthread_rwlock_unlock(&rw); /* allocates the lock, for the watchdog */
	}
	pthread_create(&c, NULL, watchdog, NULL);
	for (i = 0; i < readers; i++)
		pthread_create(&t[i], NULL, reader, NULL);
	for (i = readers; i < readers + writers; i++)
		pthread_create(&t[i], NULL, writer, NULL);
	for (i = 0; i < readers + writers; i++)
		pthread_join(t[i], &r);
	printf("thr-test: rwlock: shared=%ld (expected %d) readers at once>1:%d\n", rw_shared, writers * 2000, rw_max_readers > 1);
	return (counter == THREADS * ROUNDS ? 0 : 1);
}

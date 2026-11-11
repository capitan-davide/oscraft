/* The calls of libsys's misc module on OsCraft: the time of day, resource
   use, alarms and POSIX timers, links, owners, times, truncation, FIFOs,
   pathconf, flock, groups and the set-id calls (last: they drop root). */
#include <sys/types.h>
#include <dirent.h>
#include <sys/file.h>
#include <sys/mman.h>
#include <sys/mount.h>
#include <sys/param.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <errno.h>
#include <sched.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

/* The entries of an open directory, read to its end. */
static int
count_entries(DIR *d)
{
	int n = 0;

	while (readdir(d) != NULL && n < 100000)
		n++;
	return (n);
}

static volatile int alarms;

static void
on_alarm(int sig)
{
	alarms += sig == SIGALRM;
}

int
main(void)
{
	struct timeval tv;
	struct timespec ts;
	struct rusage ru;
	struct itimerval itv;
	struct itimerspec its;
	struct sigevent sev;
	struct stat st;
	sigset_t set;
	siginfo_t info;
	timer_t tid;
	char buf[64];
	const char *dir = "/dev/shmem";
	int fd, r;
	gid_t groups[8];

	gettimeofday(&tv, NULL);
	clock_gettime(CLOCK_REALTIME, &ts);
	printf("misc: gettimeofday within 1s of clock_gettime:%d\n", ts.tv_sec - tv.tv_sec <= 1);
	printf("misc: sched_yield=%d\n", sched_yield());
	r = getrusage(RUSAGE_SELF, &ru);
	printf("misc: getrusage=%d utime>=0:%d\n", r, ru.ru_utime.tv_sec >= 0);

	signal(SIGALRM, on_alarm);
	itv.it_value.tv_sec = 0;
	itv.it_value.tv_usec = 20000;
	itv.it_interval.tv_sec = 0;
	itv.it_interval.tv_usec = 0;
	printf("misc: setitimer=%d", setitimer(ITIMER_REAL, &itv, NULL));
	getitimer(ITIMER_REAL, &itv);
	printf(" getitimer left>0:%d", itv.it_value.tv_usec > 0 || itv.it_value.tv_sec > 0);
	usleep(100000);
	printf(" alarms=%d", alarms);
	alarm(1);
	printf(" alarm(1) pending:%d\n", alarm(0) == 1);

	sigemptyset(&set);
	sigaddset(&set, SIGUSR1);
	sigprocmask(SIG_BLOCK, &set, NULL);
	memset(&sev, 0, sizeof sev);
	sev.sigev_notify = SIGEV_SIGNAL;
	sev.sigev_signo = SIGUSR1;
	sev.sigev_value.sival_int = 7;
	printf("misc: timer_create=%d", timer_create(CLOCK_MONOTONIC, &sev, &tid));
	its.it_value.tv_sec = 0;
	its.it_value.tv_nsec = 30000000;
	its.it_interval.tv_sec = 0;
	its.it_interval.tv_nsec = 30000000;
	printf(" settime=%d", timer_settime(tid, 0, &its, NULL));
	timer_gettime(tid, &its);
	printf(" gettime left<=30ms:%d interval=30ms:%d", its.it_value.tv_sec == 0 && its.it_value.tv_nsec <= 30000000, its.it_interval.tv_nsec == 30000000);
	r = sigwaitinfo(&set, &info);
	printf(" signal=%d value=%d code=SI_TIMER:%d", r == SIGUSR1, info.si_value.sival_int, info.si_code == SI_TIMER);
	usleep(100000);
	printf(" overrun>0:%d", timer_getoverrun(tid) > 0);
	printf(" delete=%d\n", timer_delete(tid));

	snprintf(buf, sizeof buf, "%s/misc-file", dir);
	fd = open(buf, O_CREAT | O_RDWR, 0644);
	write(fd, "0123456789", 10);
	printf("misc: truncate=%d", truncate(buf, 4));
	fstat(fd, &st);
	printf(" size=%lld", (long long)st.st_size);
	printf(" fchown(0,0)=%d", fchown(fd, 0, 0));
	ts.tv_sec = 1000000;
	ts.tv_nsec = 0;
	{
		struct timespec two[2] = { ts, ts };
		printf(" futimens=%d", futimens(fd, two));
	}
	fstat(fd, &st);
	printf(" mtime=1000000:%d", st.st_mtime == 1000000);
	{
		struct timespec two[2] = { { 0, UTIME_NOW }, { 0, UTIME_OMIT } };
		printf(" utimensat(now,omit)=%d", utimensat(AT_FDCWD, buf, two, 0));
	}
	fstat(fd, &st);
	printf(" mtime kept:%d", st.st_mtime == 1000000);
	r = fpathconf(fd, _PC_NAME_MAX);
	printf(" fpathconf(NAME_MAX)=%d (errno %d)", r, r < 0 ? errno : 0);
	r = pathconf("/proc/boot", _PC_NAME_MAX);
	printf(" pathconf(/proc/boot, NAME_MAX)=%d (errno %d)", r, r < 0 ? errno : 0);
	r = flock(fd, LOCK_EX | LOCK_NB);
	printf(" flock=%d (errno %d)", r, r ? errno : 0);
	if (r == 0)
		printf(" unlock=%d", flock(fd, LOCK_UN));
	printf("\n");
	close(fd);

	snprintf(buf, sizeof buf, "%s/misc-link", dir);
	unlink(buf);
	/* /dev/shmem keeps no links: the request reaches it and is refused. */
	r = symlink("misc-file", buf);
	printf("misc: symlink=%d (errno %d)", r, r ? errno : 0);
	if (r == 0) {
		r = readlink(buf, buf + 32, 31);
		printf(" readlink=%.*s", r > 0 ? r : 0, buf + 32);
		printf(" lstat link:%d", lstat(buf, &st) == 0 && S_ISLNK(st.st_mode));
		unlink(buf);
	}
	printf("\n");

	snprintf(buf, sizeof buf, "%s/misc-fifo", dir);
	/* nor FIFO nodes */
	r = mkfifo(buf, 0600);
	printf("misc: mkfifo=%d (errno %d)", r, r ? errno : 0);
	if (r == 0) {
		printf(" fifo:%d", stat(buf, &st) == 0 && S_ISFIFO(st.st_mode));
		unlink(buf);
	}
	printf("\n");
	{
		char page[4096 * 2];
		unsigned char vec[3];
		printf("misc: mincore=%d mlock=%d msync=%d\n", mincore(page, sizeof page, (char *)vec), mlock(page, sizeof page), msync(page, sizeof page, MS_SYNC));
	}

	{
		struct statfs sf;
		struct sched_param sp;
		stack_t ss, oss;
		char login[32];
		r = statfs("/dev/shmem", &sf);
		printf("misc: statfs(/dev/shmem)=%d (errno %d)", r, r ? errno : 0);
		if (r == 0)
			printf(" bsize>0:%d type=%s", sf.f_bsize > 0, sf.f_fstypename);
		printf(" getpriority=%d", getpriority(PRIO_PROCESS, 0));
		printf(" setpriority(5)=%d now=%d", setpriority(PRIO_PROCESS, 0, 5), getpriority(PRIO_PROCESS, 0));
		setpriority(PRIO_PROCESS, 0, 0);
		printf(" sched_getscheduler=%d (OTHER=%d)", sched_getscheduler(0), SCHED_OTHER);
		sp.sched_priority = 3;
		printf(" setscheduler(FIFO,3)=%d", sched_setscheduler(0, SCHED_FIFO, &sp));
		sched_getparam(0, &sp);
		printf(" now=%d prio=%d\n", sched_getscheduler(0), sp.sched_priority);
		ss.ss_sp = login;
		ss.ss_size = 8192;
		ss.ss_flags = 0;
		printf("misc: sigaltstack set=%d", sigaltstack(&ss, NULL));
		memset(&oss, 0, sizeof oss);
		sigaltstack(NULL, &oss);
		printf(" get size=%zu", oss.ss_size);
		fd = shm_open("/misc-shm", O_CREAT | O_RDWR, 0600);
		printf(" shm_open=%d", fd >= 0);
		ftruncate(fd, 4096);
		{
			char *m = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
			if (m != MAP_FAILED) {
				strcpy(m, "shm");
				printf(" mapped=%s", m);
				munmap(m, 4096);
			}
		}
		close(fd);
		printf(" shm_unlink=%d", shm_unlink("/misc-shm"));
		r = getlogin_r(login, sizeof login);
		printf(" getlogin=%s\n", r == 0 ? login : "-");
		/* A real-time signal QNX has no number for: queued, with its
		 * value, by FreeBSD's numbers and siginfo_t. */
		sigset_t rt;
		siginfo_t si;
		union sigval sv = {.sival_int = 7};
		int rtsig = SIGRTMIN + 40;
		sigemptyset(&rt);
		sigaddset(&rt, rtsig);
		sigprocmask(SIG_BLOCK, &rt, NULL);
		sigqueue(getpid(), rtsig, sv);
		sv.sival_int = 8;
		sigqueue(getpid(), rtsig, sv);
		int first_rt = sigwaitinfo(&rt, &si);
		int first_val = si.si_value.sival_int;
		int second_rt = sigwaitinfo(&rt, &si);
		printf("misc: rt signal queued:%d values:%d,%d\n", first_rt == rtsig && second_rt == rtsig,
		    first_val, si.si_value.sival_int);
		sigprocmask(SIG_UNBLOCK, &rt, NULL);

		/* SA_RESTART: a read a handled signal interrupts goes on. */
		int rp[2];
		pipe(rp);
		pid_t writer = fork();
		if (writer == 0) {
			usleep(300000);
			write(rp[1], "r", 1);
			_exit(0);
		}
		struct sigaction ra = {.sa_handler = on_alarm, .sa_flags = SA_RESTART};
		sigemptyset(&ra.sa_mask);
		sigaction(SIGALRM, &ra, NULL);
		pid_t me = getpid();
		pid_t poker = fork();
		if (poker == 0) {
			usleep(100000);
			kill(me, SIGALRM);
			_exit(0);
		}
		int before = alarms;
		char rb;
		ssize_t rn = read(rp[0], &rb, 1);
		printf("misc: SA_RESTART handler ran:%d read:%d\n", alarms == before + 1, (int)rn);
		waitpid(writer, NULL, 0);
		waitpid(poker, NULL, 0);
		close(rp[0]);
		close(rp[1]);

		/* A directory ends, and rewinddir() starts it again. */
		DIR *dir = opendir("/bin");
		int first = count_entries(dir);
		rewinddir(dir);
		printf("misc: readdir ends:%d rewinddir same:%d\n", first > 0 && first < 100000,
		    count_entries(dir) == first);
		closedir(dir);
	}
	r = getgroups(8, groups);
	printf("misc: getgroups=%d first=%d", r, r > 0 ? (int)groups[0] : -1);
	groups[0] = 5;
	groups[1] = 6;
	printf(" setgroups=%d", setgroups(2, groups));
	r = getgroups(8, groups);
	printf(" now %d: %d %d", r, (int)groups[0], (int)groups[1]);
	printf(" setegid(7)=%d egid=%d", setegid(7), (int)getegid());
	printf(" setuid(100)=%d uid=%d euid=%d", setuid(100), (int)getuid(), (int)geteuid());
	printf(" seteuid(0) as 100: %d\n", seteuid(0));
	printf("misc: done\n");
	return (0);
}

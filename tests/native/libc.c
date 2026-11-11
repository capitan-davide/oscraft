/* A FreeBSD program with FreeBSD's libc, on OsCraft: stdio, malloc, the
   environment, errno, and an exit status. */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <time.h>
#include <signal.h>
#include <ucontext.h>

static volatile int got_signo, got_code, got_pid_ok, got_elr_ok;

static void
on_usr1(int sig, siginfo_t *si, void *ctx)
{
	ucontext_t *uc = ctx;

	got_signo = sig;
	got_code = si->si_code;
	got_pid_ok = si->si_pid == getpid();
	got_elr_ok = uc->uc_mcontext.mc_gpregs.gp_elr != 0 &&
	    uc->uc_mcontext.mc_gpregs.gp_sp != 0;
}

int
main(int argc, char *argv[])
{
	char *p;
	int i;

	printf("libc-test: hello from FreeBSD's libc on OsCraft, argc=%d\n", argc);
	for (i = 0; i < argc; i++)
		printf("libc-test: argv[%d]=%s\n", i, argv[i]);
	p = malloc(100);
	if (p == NULL) {
		printf("libc-test: malloc failed\n");
		return (2);
	}
	strcpy(p, "a string in malloc'ed memory");
	printf("libc-test: %s (%zu bytes)\n", p, strlen(p));
	free(p);
	printf("libc-test: PATH=%s\n", getenv("PATH") ? getenv("PATH") : "(unset)");
	printf("libc-test: getpid=%d getpagesize=%d\n", getpid(), getpagesize());
	errno = 0;
	if (write(99, "x", 1) == -1)
		printf("libc-test: write(99): %s\n", strerror(errno));
	fflush(stdout);
	/* Files: write one, read it back, stat it, remove it. */
	{
		FILE *f;
		char line[80];
		struct stat st;
		struct timespec ts;

		f = fopen("/tmp/libc-test.txt", "w");
		if (f == NULL) {
			printf("libc-test: fopen(w): %s\n", strerror(errno));
			return (3);
		}
		fprintf(f, "line one\nline two\n");
		fclose(f);
		f = fopen("/tmp/libc-test.txt", "r");
		if (f == NULL) {
			printf("libc-test: fopen(r): %s\n", strerror(errno));
			return (4);
		}
		while (fgets(line, sizeof(line), f) != NULL)
			printf("libc-test: read: %s", line);
		fclose(f);
		if (stat("/tmp/libc-test.txt", &st) == 0)
			printf("libc-test: stat: size=%lld mode=%o\n", (long long)st.st_size, st.st_mode & 0777);
		else
			printf("libc-test: stat: %s\n", strerror(errno));
		printf("libc-test: access(/proc/boot, R_OK)=%d access(/nonexistent, F_OK)=%d errno=%s\n",
		    access("/proc/boot", R_OK), access("/nonexistent", F_OK), strerror(errno));
		printf("libc-test: uid=%d gid=%d ppid>0:%d\n", getuid(), getgid(), getppid() > 0);
		if (clock_gettime(CLOCK_MONOTONIC, &ts) == 0)
			printf("libc-test: monotonic>0:%d\n", ts.tv_sec > 0 || ts.tv_nsec > 0);
		printf("libc-test: ncpu=%ld\n", sysconf(_SC_NPROCESSORS_ONLN));
		{
			struct sigaction sa, osa;
			sigset_t set, old;

			sa.sa_sigaction = on_usr1;
			sa.sa_flags = SA_SIGINFO;
			sigemptyset(&sa.sa_mask);
			if (sigaction(SIGUSR1, &sa, &osa) == 0 && kill(getpid(), SIGUSR1) == 0)
				printf("libc-test: SIGUSR1 handler: signo=%d code=SI_USER:%d pid ok:%d context ok:%d old handler dfl:%d\n",
				    got_signo, got_code == SI_USER, got_pid_ok, got_elr_ok, osa.sa_handler == SIG_DFL);
			else
				printf("libc-test: sigaction/kill failed: %s\n", strerror(errno));

			sigemptyset(&set);
			sigaddset(&set, SIGUSR1);
			sigprocmask(SIG_BLOCK, &set, NULL);
			sigprocmask(SIG_SETMASK, NULL, &old);
			printf("libc-test: SIGUSR1 blocked:%d SIGINT blocked:%d kill(self,0)=%d\n",
			    sigismember(&old, SIGUSR1), sigismember(&old, SIGINT), kill(getpid(), 0));
			sigprocmask(SIG_UNBLOCK, &set, NULL);
		}
		if (rename("/tmp/libc-test.txt", "/tmp/renamed-test.txt") == 0 && stat("/tmp/renamed-test.txt", &st) == 0 &&
		    stat("/tmp/libc-test.txt", &st) == -1)
			printf("libc-test: rename ok, old gone: %s\n", strerror(errno));
		else
			printf("libc-test: rename: %s\n", strerror(errno));
		if (unlink("/tmp/renamed-test.txt") == -1)
			printf("libc-test: unlink: %s\n", strerror(errno));
	}
	return (argc == 3 ? 7 : 1);
}

/* FreeBSD's fork, execve and wait on OsCraft: a child that exits, a
   child that execs another program, a child that is killed. */
#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

int
main(void)
{
	pid_t me = getpid(), child, got;
	int status;
	char *const argv[] = { "hello-test", "one", "two", NULL };

	printf("fork-test: parent pid=%d\n", me);
	fflush(stdout);
	child = fork();
	if (child == -1) {
		printf("fork-test: fork: %s\n", strerror(errno));
		return (2);
	}
	if (child == 0) {
		printf("fork-test: child pid>parent:%d getppid==parent:%d\n", getpid() > me, getppid() == me);
		fflush(stdout);
		_exit(3);
	}
	got = waitpid(child, &status, 0);
	printf("fork-test: waited pid ok:%d exited:%d status=%d\n", got == child, WIFEXITED(status), WEXITSTATUS(status));

	child = fork();
	if (child == 0) {
		execv("/proc/boot/hello-test", argv);
		printf("fork-test: execv: %s\n", strerror(errno));
		_exit(4);
	}
	got = waitpid(child, &status, 0);
	printf("fork-test: exec'ed child exited:%d status=%d\n", WIFEXITED(status), WEXITSTATUS(status));

	child = fork();
	if (child == 0) {
		pause();
		_exit(5);
	}
	kill(child, SIGKILL);
	got = waitpid(child, &status, 0);
	printf("fork-test: killed child signaled:%d sig=SIGKILL:%d\n", WIFSIGNALED(status), WTERMSIG(status) == SIGKILL);
	return (0);
}

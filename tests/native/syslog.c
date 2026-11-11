/* syslog(3) to syslogd and its log file, FreeBSD's semantics: selectors in
   /etc/syslog.conf choose what is written, the line has the time, host and
   tag, and a message survives its process ending at once.
   Prints one "ok"/"FAIL" line per check and "syslog-test: passed". */
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <poll.h>
#include <spawn.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <syslog.h>
#include <unistd.h>

extern char **environ;

static int fails;

static void
check(const char *what, int ok)
{
	printf("syslog-test: %s %s\n", ok ? "ok" : "FAIL", what);
	fails += !ok;
}

static char *
logged(const char *what)
{
	static char buf[65536];
	for (int i = 0; i < 100; i++) {
		int fd = open("/var/log/messages", O_RDONLY);
		ssize_t n = fd < 0 ? 0 : read(fd, buf, sizeof buf - 1);
		if (fd >= 0)
			close(fd);
		buf[n > 0 ? n : 0] = 0;
		char *p = strstr(buf, what);
		if (p != NULL)
			return p;
		usleep(10000);
	}
	return NULL;
}

static void
write_file(const char *path, const char *text)
{
	int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
	write(fd, text, strlen(text));
	close(fd);
}

static char *
file_has(const char *path, const char *what)
{
	static char buf[8192];
	for (int i = 0; i < 100; i++) {
		int fd = open(path, O_RDONLY);
		ssize_t n = fd < 0 ? 0 : read(fd, buf, sizeof buf - 1);
		if (fd >= 0)
			close(fd);
		buf[n > 0 ? n : 0] = 0;
		char *p = strstr(buf, what);
		if (p != NULL)
			return p;
		usleep(10000);
	}
	return NULL;
}

/* syslogd on another configuration, as long as `body` runs. */
static void
with_conf(const char *conf, void (*body)(void))
{
	char ps[16] = { 0 };
	int pf = open("/var/run/syslog.pid", O_RDONLY);
	read(pf, ps, sizeof ps - 1);
	close(pf);
	kill(atoi(ps), SIGTERM);
	usleep(200000);
	write_file("/tmp/sl.conf", conf);
	pid_t d = fork();
	if (d == 0) {
		execl("/proc/boot/syslogd", "syslogd", "-f", "/tmp/sl.conf", (char *)NULL);
		_exit(1);
	}
	usleep(300000);
	body();
	kill(d, SIGTERM);
	waitpid(d, NULL, 0);
	if (fork() == 0) {
		execl("/proc/boot/syslogd", "syslogd", (char *)NULL);
		_exit(1);
	}
	usleep(300000);
}

static void
conf_body(void)
{
	openlog("sltest4", 0, LOG_USER);
	syslog(LOG_NOTICE, "to the program");
	check("|program", file_has("/tmp/piped", "to the program") != NULL);
	check("a program block", file_has("/tmp/blocked", "to the program") != NULL);
	openlog("other", 0, LOG_USER);
	syslog(LOG_NOTICE, "disk trouble");
	check("a property filter", file_has("/tmp/filtered", "disk trouble") != NULL);
	check("include", file_has("/tmp/included", "disk trouble") != NULL);
	check("a regular expression", file_has("/tmp/regex", "disk trouble") != NULL);
	syslog(LOG_NOTICE, "not disk trouble");
	check("it doesn't match", file_has("/tmp/included", "not disk trouble") != NULL &&
	    file_has("/tmp/regex", "not disk trouble") == NULL);
	check("not in the block", file_has("/tmp/blocked", "disk trouble") == NULL);
	closelog();
}

static void
forward_body(void)
{
	int u = socket(AF_INET, SOCK_DGRAM, 0);
	struct sockaddr_in in = { .sin_family = AF_INET, .sin_port = htons(5514) };
	in.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
	bind(u, (struct sockaddr *)&in, sizeof in);
	openlog("sltest5", 0, LOG_USER);
	syslog(LOG_NOTICE, "over the network");
	struct pollfd pf = { u, POLLIN, 0 };
	char b[512] = { 0 };
	check("@host: a datagram", poll(&pf, 1, 3000) == 1 && recv(u, b, sizeof b - 1, 0) > 0);
	check("@host: the classic form", strncmp(b, "<13>", 4) == 0 && strstr(b, " sltest5[") != NULL &&
	    strstr(b, "over the network") != NULL);
	closelog();
	close(u);
}

int
main(int argc, char **argv)
{
	if (argc > 1 && strcmp(argv[1], "conf") == 0) {
		mkdir("/tmp/slinc", 0755);
		write_file("/tmp/slinc/a.conf", "*.notice /tmp/included\n");
		with_conf("include /tmp/slinc\n*.notice |/bin/cat >> /tmp/piped\n"
		    "!sltest4\n*.notice /tmp/blocked\n!*\n"
		    ":msg, contains, \"disk\"\n*.notice /tmp/filtered\n"
		    ":msg, ereregex, \"^(disk|net) trouble$\"\n*.notice /tmp/regex\n", conf_body);
		printf("syslog-test: %s\n", fails ? "FAILED" : "passed");
		return fails != 0;
	}
	if (argc > 1 && strcmp(argv[1], "spawn") == 0) {
		char *cat[] = { "/bin/echo", "spawned", NULL };
		pid_t pid;
		int status = -1;
		int e = posix_spawn(&pid, "/bin/echo", NULL, NULL, cat, environ);
		check("posix_spawn()", e == 0 && waitpid(pid, &status, 0) == pid && WIFEXITED(status) && WEXITSTATUS(status) == 0);
		printf("syslog-test: %s\n", fails ? "FAILED" : "passed");
		return fails != 0;
	}
	if (argc > 1 && strcmp(argv[1], "forward") == 0) {
		with_conf("*.notice @127.0.0.1:5514\n", forward_body);
		printf("syslog-test: %s\n", fails ? "FAILED" : "passed");
		return fails != 0;
	}
	openlog("sltest", LOG_PID, LOG_USER);
	syslog(LOG_NOTICE, "notice one");
	char *p = logged("notice one");
	check("a notice reaches the file", p != NULL);
	if (p != NULL) {
		char *line = p;
		while (line > p - 200 && line[-1] != '\n' && line[-1] != 0)
			line--;
		char want[300], host[256] = { 0 };
		gethostname(host, sizeof host - 1);
		snprintf(want, sizeof want, " %s sltest[", host);
		check("the host and the tag", strstr(line, want) != NULL);
	}
	syslog(LOG_INFO, "info below the selector");
	syslog(LOG_WARNING, "warning after it");
	check("a later one goes through", logged("warning after it") != NULL);
	check("info isn't selected", logged("info below the selector") == NULL);

	if (fork() == 0) {
		openlog("sltest2", 0, LOG_DAEMON);
		syslog(LOG_ERR, "from a short life");
		_exit(0);
	}
	wait(NULL);
	check("a process's last words", logged("from a short life") != NULL);

	if (fork() == 0) {
		setuid(1000);
		openlog("sltest3", 0, LOG_USER);
		syslog(LOG_NOTICE, "from an ordinary user");
		_exit(0);
	}
	wait(NULL);
	check("an ordinary user logs", logged("from an ordinary user") != NULL);

	/* Registered as one user, the ring is closed to another. */
	if (fork() == 0) {
		char r[128] = { 0 };
		seteuid(1000);
		int f = open("/var/run/log", O_RDWR);
		read(f, r, sizeof r - 1);
		seteuid(0);
		*strchr(r, ' ') = 0;
		setuid(1001);
		_exit(open(r, O_RDWR) < 0 && errno == EACCES ? 0 : 1);
	}
	int status;
	wait(&status);
	check("another user can't open a ring", WIFEXITED(status) && WEXITSTATUS(status) == 0);

	/* syslogd restarts: what is written meanwhile isn't lost. */
	int pf = open("/var/run/syslog.pid", O_RDONLY);
	char ps[16] = { 0 };
	read(pf, ps, sizeof ps - 1);
	close(pf);
	syslog(LOG_NOTICE, "before the restart");
	check("before the restart", logged("before the restart") != NULL);
	check("kill(syslogd)", kill(atoi(ps), SIGKILL) == 0);
	usleep(100000);
	syslog(LOG_NOTICE, "while it is down");
	if (fork() == 0) {
		execl("/proc/boot/syslogd", "syslogd", (char *)NULL);
		_exit(1);
	}
	check("written while it was down", logged("while it is down") != NULL);
	syslog(LOG_NOTICE, "after the restart one");
	usleep(100000);
	syslog(LOG_NOTICE, "after the restart two");
	check("after it", logged("after the restart two") != NULL);
	char *once = logged("while it is down");
	check("nothing twice", once != NULL && strstr(once + 1, "while it is down") == NULL);
	once = logged("before the restart");
	check("nothing old again", once != NULL && strstr(once + 1, "before the restart") == NULL);

	/* Repeats are counted, and SIGHUP reopens the logs. */
	syslog(LOG_NOTICE, "over and over");
	syslog(LOG_NOTICE, "over and over");
	syslog(LOG_NOTICE, "over and over");
	syslog(LOG_NOTICE, "something else");
	check("repeats are counted", logged("last message repeated 2 times") != NULL);
	pf = open("/var/run/syslog.pid", O_RDONLY);
	memset(ps, 0, sizeof ps);
	read(pf, ps, sizeof ps - 1);
	close(pf);
	rename("/var/log/messages", "/var/log/messages.0");
	check("SIGHUP", kill(atoi(ps), SIGHUP) == 0);
	usleep(200000);
	syslog(LOG_NOTICE, "in the new log");
	check("a new log after SIGHUP", logged("in the new log") != NULL);
	int rot = open("/var/log/messages.0", O_RDONLY);
	static char old[65536];
	read(rot, old, sizeof old - 1);
	close(rot);
	check("the old one kept what it had", strstr(old, "something else") != NULL);

	closelog();
	syslog(LOG_NOTICE, "after closelog");
	check("closelog(), then a new connection", logged("after closelog") != NULL);
	printf("syslog-test: %s\n", fails ? "FAILED" : "passed");
	return fails != 0;
}

/* POSIX message queues (with QNX's mqueue server running): messages come
 * out by priority, then in order; attributes; non-blocking calls; a
 * receiver blocked until a send; a full queue (EAGAIN, ETIMEDOUT);
 * EMSGSIZE; mq_notify(); a queue shared with another process; unlinking.
 * Prints one "ok"/"FAIL" line per case. */
#include <errno.h>
#include <fcntl.h>
#include <mqueue.h>
#include <pthread.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define NAME "/mqtest"

static int fails;

static void check(const char *what, int ok)
{
    printf("mqtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void *late_sender(void *arg)
{
    usleep(100 * 1000);
    mq_send(*(mqd_t *)arg, "late", 5, 3);
    return NULL;
}

int main(void)
{
    mq_unlink(NAME);
    struct mq_attr attr = {.mq_maxmsg = 4, .mq_msgsize = 64};
    mqd_t q = mq_open(NAME, O_CREAT | O_EXCL | O_RDWR, 0600, &attr);
    check("mq_open() creates", q != (mqd_t)-1);
    if (q == (mqd_t)-1) {
        printf("mqtest: errno %d\n", errno);
        printf("mqtest: FAILED\n");
        return 1;
    }

    int sent = mq_send(q, "low", 4, 1) == 0 && mq_send(q, "high", 5, 9) == 0 &&
               mq_send(q, "low2", 5, 1) == 0;
    struct mq_attr got;
    mq_getattr(q, &got);
    check("sends, mq_getattr()", sent && got.mq_curmsgs == 3 && got.mq_maxmsg == 4 &&
                                     got.mq_msgsize == 64);

    char buf[64];
    unsigned prio = 0;
    ssize_t n1 = mq_receive(q, buf, sizeof buf, &prio);
    int first = n1 == 5 && !strcmp(buf, "high") && prio == 9;
    ssize_t n2 = mq_receive(q, buf, sizeof buf, &prio);
    int second = n2 == 4 && !strcmp(buf, "low") && prio == 1;
    ssize_t n3 = mq_receive(q, buf, sizeof buf, &prio);
    check("by priority, then in order", first && second && n3 == 5 && !strcmp(buf, "low2"));

    check("EMSGSIZE for a small buffer", mq_receive(q, buf, 10, NULL) == -1 && errno == EMSGSIZE);
    char big[100] = {0};
    check("EMSGSIZE for a big message", mq_send(q, big, sizeof big, 0) == -1 && errno == EMSGSIZE);

    struct mq_attr nb = {.mq_flags = O_NONBLOCK}, old;
    mq_setattr(q, &nb, &old);
    check("non-blocking receive: EAGAIN",
          mq_receive(q, buf, sizeof buf, NULL) == -1 && errno == EAGAIN);
    for (int i = 0; i < 4; i++)
        mq_send(q, "x", 2, 0);
    check("non-blocking send to a full queue: EAGAIN",
          mq_send(q, "y", 2, 0) == -1 && errno == EAGAIN);
    nb.mq_flags = 0;
    mq_setattr(q, &nb, NULL);
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    t.tv_nsec += 100 * 1000 * 1000;
    if (t.tv_nsec >= 1000000000) {
        t.tv_sec++;
        t.tv_nsec -= 1000000000;
    }
    check("timed send to a full queue: ETIMEDOUT",
          mq_timedsend(q, "y", 2, 0, &t) == -1 && errno == ETIMEDOUT);
    for (int i = 0; i < 4; i++)
        mq_receive(q, buf, sizeof buf, NULL);

    pthread_t th;
    pthread_create(&th, NULL, late_sender, &q);
    ssize_t n = mq_receive(q, buf, sizeof buf, &prio);
    pthread_join(th, NULL);
    check("blocked receive, woken by a send", n == 5 && !strcmp(buf, "late") && prio == 3);

    sigset_t set;
    sigemptyset(&set);
    sigaddset(&set, SIGUSR1);
    sigprocmask(SIG_BLOCK, &set, NULL);
    struct sigevent ev;
    SIGEV_SIGNAL_INIT(&ev, SIGUSR1);
    int rc = mq_notify(q, &ev);
    mq_send(q, "note", 5, 0);
    struct timespec wait = {1, 0};
    check("mq_notify()", rc == 0 && sigtimedwait(&set, NULL, &wait) == SIGUSR1);
    mq_receive(q, buf, sizeof buf, NULL);

    pid_t pid = fork();
    if (pid == 0) {
        mqd_t c = mq_open(NAME, O_WRONLY);
        int ok = c != (mqd_t)-1 && mq_send(c, "child", 6, 5) == 0;
        mq_close(c);
        _exit(ok ? 0 : 1);
    }
    n = mq_receive(q, buf, sizeof buf, &prio);
    int status = -1;
    waitpid(pid, &status, 0);
    check("another process's message",
          WIFEXITED(status) && WEXITSTATUS(status) == 0 && n == 6 && !strcmp(buf, "child"));

    check("mq_close()", mq_close(q) == 0);
    check("mq_unlink()", mq_unlink(NAME) == 0);
    check("gone after unlink", mq_open(NAME, O_RDONLY) == (mqd_t)-1 && errno == ENOENT);

    printf("mqtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

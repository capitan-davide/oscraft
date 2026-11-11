/* Memory given back: free pages (sysconf(_SC_AVPHYS_PAGES)) before and
 * after many spawned processes, forked ones, threads, channels and
 * connections, after a warm-up round of each. `leaktest exit` just exits
 * (the program spawn() starts). Prints the pages each case keeps, and
 * "ok"/"FAIL" (more than a little slack kept). */
#include <pthread.h>
#include <spawn.h>
#include <signal.h>
#include <stdio.h>
#include <sys/mman.h>
#include <string.h>
#include <sys/neutrino.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;
static int fails;
static char *self;

/* Free pages once the deferred frees are done (procnto's work on ended
 * processes, dead threads' stacks): read until three readings in a row
 * find no more (at most 5 s). */
static long free_pages(void)
{
    long most = sysconf(_SC_AVPHYS_PAGES);
    for (int same = 0, i = 0; same < 3 && i < 100; i++) {
        usleep(50 * 1000);
        long n = sysconf(_SC_AVPHYS_PAGES);
        same = n > most ? 0 : same + 1;
        most = n > most ? n : most;
    }
    return most;
}

static void spawn_one(void)
{
    char *args[] = {self, "exit", NULL};
    pid_t pid;
    if (posix_spawn(&pid, self, NULL, NULL, args, environ) == 0)
        waitpid(pid, NULL, 0);
}

static void fork_one(void)
{
    pid_t pid = fork();
    if (pid == 0)
        _exit(0);
    waitpid(pid, NULL, 0);
}

static void *nothing(void *arg)
{
    return arg;
}

static void thread_one(void)
{
    pthread_t t;
    pthread_create(&t, NULL, nothing, NULL);
    pthread_join(t, NULL);
}

static void channel_one(void)
{
    int chid = ChannelCreate(0);
    int coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    ConnectDetach(coid);
    ChannelDestroy(chid);
}

static void channel_process_one(void)
{
    pid_t pid = fork();
    if (pid == 0) {
        ChannelCreate(0); /* left for the process's end to free */
        _exit(0);
    }
    waitpid(pid, NULL, 0);
}

/* Anonymous shared memory, written by a forked child, then unmapped. */
static void shared_anon_one(void)
{
    size_t len = 16 * 4096;
    char *m = mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON, NOFD, 0);
    pid_t pid = fork();
    if (pid == 0) {
        m[0] = 1;
        _exit(0);
    }
    waitpid(pid, NULL, 0);
    munmap(m, len);
}

static void sigev_notify_fn(union sigval v)
{
    (void)v;
}

/* A SIGEV_THREAD timer's expiry: a thread on a stack the kernel maps. */
static void sigev_thread_one(void)
{
    struct sigevent ev;
    memset(&ev, 0, sizeof ev);
    SIGEV_THREAD_INIT(&ev, sigev_notify_fn, 0, NULL);
    timer_t t;
    timer_create(CLOCK_MONOTONIC, &ev, &t);
    struct itimerspec its = { .it_value = { 0, 1000000 } };
    timer_settime(t, 0, &its, NULL);
    usleep(5000);
    timer_delete(t);
}

/* Pages kept by n runs of `fn`. */
static long round_of(void (*fn)(void), int n)
{
    long before = free_pages();
    for (int i = 0; i < n; i++)
        fn();
    return before - free_pages();
}

/* Run `fn` n times, after a warm-up round of as many, twice: a leak keeps
 * pages in every round, a cache (the kernel heap's free lists, work left
 * for later) not; `slack` pages may stay. */
static void measure(const char *what, void (*fn)(void), int n, long slack)
{
    for (int i = 0; i < n; i++)
        fn();
    long kept1 = round_of(fn, n);
    long kept2 = round_of(fn, n);
    int ok = kept1 <= slack || kept2 <= slack;
    printf("leaktest: %s %s (%d times, twice: %ld, %ld pages kept)\n", ok ? "ok" : "FAIL", what,
           n, kept1, kept2);
    fails += !ok;
}

int main(int argc, char **argv)
{
    if (argc > 1 && !strcmp(argv[1], "exit"))
        return 0;
    self = argv[0];
    /* Right after boot, other processes (the network stack, dhcpcd) still
     * allocate: wait until free memory holds still for two seconds (20 s
     * at most), so that their pages don't count as ours. */
    for (int i = 0, still = 0; i < 20 && still < 2; i++) {
        long before = free_pages();
        sleep(1);
        still = free_pages() == before ? still + 1 : 0;
    }
    measure("spawn()", spawn_one, 200, 3);
    measure("fork()", fork_one, 200, 3);
    measure("threads", thread_one, 1000, 3);
    measure("channels and connections", channel_one, 1000, 3);
    measure("a process's channel", channel_process_one, 200, 3);
    measure("shared anonymous memory across fork()", shared_anon_one, 100, 3);
    measure("SIGEV_THREAD threads", sigev_thread_one, 100, 3);
    printf("leaktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

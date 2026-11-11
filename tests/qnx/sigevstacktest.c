/* The stack the kernel makes for a SIGEV_THREAD thread (review item
 * SCHED-7), as procnto (measured): a MAP_STACK mapping of the attribute's
 * stack size (256 KiB by default), with a 4 KiB guard page right below
 * it, no access (MAP_STACK too), so that an overflow faults. The kernel
 * used to map the stack alone, without MAP_STACK and without a guard.
 * Checked from inside the thread, through /proc/self/as's mappings, with
 * the default attributes and with a stack size given. Prints one
 * "ok"/"FAIL" line per check and "sigevstacktest: passed". */
#include <devctl.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procfs.h>
#include <time.h>
#include <unistd.h>

#define KEEP (MAP_STACK | MAP_ANON | MAP_PRIVATE | PROT_READ | PROT_WRITE | PROT_EXEC)

static int fails;
static volatile uintptr_t where;
static volatile int done;

static void check(const char *what, int ok)
{
    printf("sigevstacktest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void fn(union sigval v)
{
    (void)v;
    volatile char local = 0;
    where = (uintptr_t)&local;
    while (!done)
        usleep(1000);
}

static procfs_mapinfo m[512];

/* run one SIGEV_THREAD thread with `attr` and check its stack's mapping
 * (`size` long) and the guard below it */
static void one(const char *what, pthread_attr_t *attr, uint64_t size)
{
    struct sigevent ev;
    memset(&ev, 0, sizeof ev);
    SIGEV_THREAD_INIT(&ev, fn, 0, attr);
    where = 0;
    done = 0;
    timer_t t;
    if (timer_create(CLOCK_MONOTONIC, &ev, &t) != 0) {
        check("timer_create", 0);
        return;
    }
    struct itimerspec it = { .it_value = { 0, 1000000 } };
    timer_settime(t, 0, &it, NULL);
    while (where == 0)
        usleep(1000);
    int fd = open("/proc/self/as", O_RDONLY);
    int n = 0;
    devctl(fd, DCMD_PROC_MAPINFO, m, sizeof m, &n);
    close(fd);
    if (n > 512)
        n = 512;
    int s = -1, g = -1;
    for (int i = 0; i < n; i++)
        if (m[i].vaddr <= where && where - m[i].vaddr < m[i].size)
            s = i;
    if (s >= 0)
        for (int i = 0; i < n; i++)
            if (m[i].vaddr + m[i].size == m[s].vaddr)
                g = i;
    char buf[128];
    snprintf(buf, sizeof buf, "%s: stack of %#llx bytes, MAP_STACK, read/write (%#llx, %#x)", what,
             (unsigned long long)size, s >= 0 ? (unsigned long long)m[s].size : 0ULL,
             s >= 0 ? m[s].flags & KEEP : 0);
    check(buf, s >= 0 && m[s].size == size &&
                   (m[s].flags & KEEP) == (MAP_STACK | MAP_ANON | MAP_PRIVATE | PROT_READ | PROT_WRITE));
    snprintf(buf, sizeof buf, "%s: a 4 KiB guard below it, MAP_STACK, no access (%#llx, %#x)", what,
             g >= 0 ? (unsigned long long)m[g].size : 0ULL, g >= 0 ? m[g].flags & KEEP : 0);
    check(buf, g >= 0 && m[g].size == 0x1000 && (m[g].flags & KEEP) == (MAP_STACK | MAP_ANON | MAP_PRIVATE));
    done = 1;
    usleep(20000);
    timer_delete(t);
}

int main(void)
{
    one("default", NULL, 0x40000);
    pthread_attr_t a;
    pthread_attr_init(&a);
    pthread_attr_setstacksize(&a, 0x20000);
    one("128 KiB", &a, 0x20000);
    printf("sigevstacktest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

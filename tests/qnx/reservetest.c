/* A large PROT_NONE mapping is a reservation: instant, and backed piece by
 * piece by mprotect() or a MAP_FIXED mapping inside it. Expected values are
 * procnto's measured results (a 64 GB reservation). */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

#define SIZE (64ul << 30)
#define CHUNK (2ul << 20)

static int fails;

static void check(const char *what, int ok)
{
    printf("reservetest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

int main(void)
{
    char *p = mmap(0, SIZE, PROT_NONE, MAP_PRIVATE | MAP_ANON, -1, 0);
    check("64 GB of PROT_NONE", p != MAP_FAILED);
    if (p == MAP_FAILED) {
        return 1;
    }
    check("mprotect() backs a piece", mprotect(p, CHUNK, PROT_READ | PROT_WRITE) == 0);
    p[0] = 1;
    p[CHUNK - 1] = 2;
    char *q = mmap(p + CHUNK, CHUNK, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0);
    check("MAP_FIXED backs the next", q == p + CHUNK);
    q[0] = 3;
    check("the pieces hold", p[0] == 1 && p[CHUNK - 1] == 2 && q[0] == 3);
    check("munmap() of it all", munmap(p, SIZE) == 0);
    printf("reservetest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

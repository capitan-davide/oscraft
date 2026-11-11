/* The resource database (rsrcdbmgr_*()), as procnto keeps it:
 * - at boot, memory is free from 0 to the end of the physical address
 *   space, and the interrupt controllers' vectors are free;
 * - create adds free ranges (merging), destroy takes the free parts away;
 * - attach takes the lowest place that fits (aligned, within a range, or
 *   the highest with TOPDOWN), all items or none; a range held is EINVAL,
 *   unless shared exactly by another process; adjacent ranges a process
 *   holds are listed as one;
 * - detach gives back (part of) a range the caller holds, EINVAL again;
 *   not once its range was destroyed;
 * - a process's ranges go back when it ends;
 * - queries: pid 0 free, -1 in use, a pid its own; EINVAL past the end;
 * - changes take PROCMGR_AID_RSRCDBMGR, queries don't.
 * Prints one "ok"/"FAIL" line per check and "rsrctest: passed". */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/procmgr.h>
#include <sys/rsrcdbmgr.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("rsrctest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static rsrc_alloc_t got[32];

/* The blocks of `name` for `pid` (0 free, -1 in use), into `got`. */
static int query(pid_t pid, const char *name)
{
    return rsrcdbmgr_query_name(got, 32, 0, pid, (char *)name, 0);
}

/* Whether the query found exactly these (start, end) pairs. */
static int blocks(int n, const unsigned long long *want, int nwant)
{
    if (n != nwant) {
        printf("rsrctest:   %d blocks:", n);
        for (int i = 0; i < n && i < 32; i++) {
            printf(" %#llx-%#llx", (unsigned long long)got[i].start, (unsigned long long)got[i].end);
        }
        printf("\n");
        return 0;
    }
    for (int i = 0; i < n; i++) {
        if (got[i].start != want[2 * i] || got[i].end != want[2 * i + 1]) {
            return blocks(-n, want, nwant);
        }
    }
    return 1;
}

static int create(unsigned flags, unsigned long long s, unsigned long long e)
{
    rsrc_alloc_t a = { .start = s, .end = e, .flags = flags };
    return rsrcdbmgr_create(&a, 1);
}

static int destroy(unsigned flags, unsigned long long s, unsigned long long e)
{
    rsrc_alloc_t a = { .start = s, .end = e, .flags = flags };
    return rsrcdbmgr_destroy(&a, 1);
}

static int attach(rsrc_request_t *r, unsigned long long len, unsigned long long s, unsigned long long e,
                  unsigned long long align, unsigned flags)
{
    memset(r, 0, sizeof *r);
    r->length = len;
    r->start = s;
    r->end = e;
    r->align = align;
    r->flags = flags;
    return rsrcdbmgr_attach(r, 1);
}

static void seeded(void)
{
    int n = query(0, "memory");
    int bits = 0;
    for (int b = 32; b <= 52; b++) {
        if (n == 1 && got[0].start == 0 && got[0].end == (1ULL << b) - 1) {
            bits = b;
        }
    }
    check("at boot, memory is free from 0 to the end of the physical address space", bits != 0);
    n = query(0, "irq");
    check("the interrupt vectors are free", n >= 1 && got[0].start == 0);
    check("nothing is in use", query(-1, "io") == 0 && query(-1, "dma") == 0);
}

static void ranges(void)
{
    const unsigned IO = RSRCDBMGR_IO_PORT;
    create(IO, 0x7000, 0x70ff);
    create(IO, 0x7080, 0x717f);
    create(IO, 0x7200, 0x72ff);
    create(IO, 0x7180, 0x71ff);
    static const unsigned long long one[] = { 0x7000, 0x72ff };
    check("created ranges merge when they overlap or touch", blocks(query(0, "io"), one, 1));

    rsrc_request_t a, b, c, d;
    attach(&a, 0x10, 0, 0, 0, IO);
    check("attach takes the lowest place", a.start == 0x7000 && a.end == 0x700f);
    attach(&b, 4, 0, 0, 0x40, IO | RSRCDBMGR_FLAG_ALIGN);
    check("aligned", b.start == 0x7040 && b.end == 0x7043);
    attach(&c, 2, 0, 0, 0, IO | RSRCDBMGR_FLAG_TOPDOWN);
    check("TOPDOWN: the highest", c.start == 0x72fe && c.end == 0x72ff);
    check("RANGE: that place", attach(&d, 1, 0x7100, 0x7100, 0, IO | RSRCDBMGR_FLAG_RANGE) == 0 && d.start == 0x7100);
    rsrc_request_t x;
    errno = 0;
    check("a place held: EINVAL",
          attach(&x, 1, 0x7100, 0x7100, 0, IO | RSRCDBMGR_FLAG_RANGE) == -1 && errno == EINVAL);
    errno = 0;
    check("too big: EINVAL", attach(&x, 0x1000, 0, 0, 0, IO) == -1 && errno == EINVAL);
    static const unsigned long long held[] = { 0x7000, 0x700f, 0x7040, 0x7043, 0x7100, 0x7100, 0x72fe, 0x72ff };
    check("in use (pid -1), in order", blocks(query(-1, "io"), held, 4));
    check("the caller's own (its pid)", blocks(query(getpid(), "io"), held, 4));
    static const unsigned long long freed[] = { 0x7010, 0x703f, 0x7044, 0x70ff, 0x7101, 0x72fd };
    check("the free blocks around them", blocks(query(0, "io"), freed, 3));

    rsrc_request_t two[2];
    memset(two, 0, sizeof two);
    two[0].length = 1;
    two[0].flags = RSRCDBMGR_IRQ;
    two[1].length = 0x1000;
    two[1].flags = IO;
    errno = 0;
    check("all items or none", rsrcdbmgr_attach(two, 2) == -1 && errno == EINVAL && query(-1, "irq") == 0);

    rsrc_request_t part = a;
    part.start = 0x7004;
    part.end = 0x7007;
    part.length = 4;
    check("detach part of a range", rsrcdbmgr_detach(&part, 1) == 0);
    static const unsigned long long split[] = { 0x7000, 0x7003, 0x7008, 0x700f, 0x7040, 0x7043, 0x7100, 0x7100, 0x72fe, 0x72ff };
    check("it splits", blocks(query(-1, "io"), split, 5));
    errno = 0;
    check("detach it again: EINVAL", rsrcdbmgr_detach(&part, 1) == -1 && errno == EINVAL);

    rsrc_request_t m1, m2;
    attach(&m1, 0x1000, 0x10000000, 0x10000fff, 0, RSRCDBMGR_MEMORY | RSRCDBMGR_FLAG_RANGE);
    attach(&m2, 0x1000, 0x10001000, 0x10001fff, 0, RSRCDBMGR_MEMORY | RSRCDBMGR_FLAG_RANGE);
    static const unsigned long long mem[] = { 0x10000000, 0x10001fff };
    check("adjacent ranges a process holds are one", blocks(query(getpid(), "memory"), mem, 1));
    rsrcdbmgr_detach(&m1, 1);
    rsrcdbmgr_detach(&m2, 1);

    pid_t p = fork();
    if (p == 0) {
        rsrc_request_t k;
        _exit(attach(&k, 8, 0x7200, 0x7207, 0, IO | RSRCDBMGR_FLAG_RANGE) != 0);
    }
    int st;
    waitpid(p, &st, 0);
    check("a process's ranges go when it ends", WEXITSTATUS(st) == 0 && blocks(query(-1, "io"), split, 5));

    rsrc_request_t s1;
    attach(&s1, 4, 0x72f0, 0x72f3, 0, IO | RSRCDBMGR_FLAG_RANGE | RSRCDBMGR_FLAG_SHARE);
    p = fork();
    if (p == 0) {
        rsrc_request_t k;
        int same = attach(&k, 4, 0x72f0, 0x72f3, 0, IO | RSRCDBMGR_FLAG_RANGE | RSRCDBMGR_FLAG_SHARE);
        int over = attach(&k, 4, 0x72f2, 0x72f5, 0, IO | RSRCDBMGR_FLAG_RANGE | RSRCDBMGR_FLAG_SHARE);
        _exit(same == 0 && over == -1 ? 0 : 1);
    }
    waitpid(p, &st, 0);
    rsrc_request_t again;
    int self = attach(&again, 4, 0x72f0, 0x72f3, 0, IO | RSRCDBMGR_FLAG_RANGE | RSRCDBMGR_FLAG_SHARE);
    check("SHARE: another process may share the range exactly, not the same one twice",
          WEXITSTATUS(st) == 0 && self == -1);
    rsrcdbmgr_detach(&s1, 1);

    rsrc_alloc_t page[2];
    errno = 0;
    check("a query past the end: EINVAL", rsrcdbmgr_query_name(page, 2, 99, -1, "io", 0) == -1 && errno == EINVAL);
    check("an unknown name: none", rsrcdbmgr_query_name(page, 2, 0, 0, "nosuch", 0) == 0);

    rsrc_request_t nm;
    memset(&nm, 0, sizeof nm);
    nm.length = 1;
    nm.start = 0x30;
    nm.end = 0x30;
    nm.flags = RSRCDBMGR_FLAG_NAME | RSRCDBMGR_FLAG_RANGE;
    nm.name = "irq";
    check("an item may name its type", rsrcdbmgr_attach(&nm, 1) == 0 && query(getpid(), "irq") == 1);
    rsrcdbmgr_detach(&nm, 1);

    check("destroy keeps what is in use", destroy(IO, 0x7000, 0x72ff) == 0 && query(0, "io") == 0 &&
                                              blocks(query(-1, "io"), split, 5));
    errno = 0;
    check("which can't be detached then", rsrcdbmgr_detach(&b, 1) == -1 && errno == EINVAL);
    check("destroy again: no error", destroy(IO, 0x7000, 0x72ff) == 0);
}

static void abilities(void)
{
    pid_t p = fork();
    if (p == 0) {
        procmgr_ability(0, PROCMGR_ADN_ROOT | PROCMGR_AOP_DENY | PROCMGR_AOP_LOCK | PROCMGR_AID_RSRCDBMGR,
                        PROCMGR_AID_EOL);
        rsrc_request_t k;
        int a = attach(&k, 0x1000, 0, 0, 0, RSRCDBMGR_MEMORY) == -1 && errno == EPERM;
        int c = create(RSRCDBMGR_DMA_CHANNEL, 0, 7) == -1 && errno == EPERM;
        int q = query(0, "irq") >= 1;
        _exit(a && c && q ? 0 : 1);
    }
    int st;
    waitpid(p, &st, 0);
    check("without RSRCDBMGR: attach and create EPERM, queries allowed", WEXITSTATUS(st) == 0);
}

int main(void)
{
    seeded();
    ranges();
    abilities();
    printf("rsrctest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

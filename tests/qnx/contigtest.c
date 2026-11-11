/* Physically contiguous memory after memory has been used and freed
 * (review item TOOLS-25, the cause of the qconn test's failures): a
 * program maps and frees most of memory, in pieces, then asks for
 * contiguous RAM below 4 GiB (mmap(MAP_PHYS | MAP_ANON), as io-sock does
 * for its buffers, and shm_ctl(SHMCTL_ANON | SHMCTL_PHYS)). procnto
 * serves them (measured); OsCraft took contiguous memory only from RAM
 * no one had used yet, and failed with ENOMEM though most of memory was
 * free. And the same in one process that goes on (PATHMEM-25b): OsCraft
 * kept the page tables of the freed mappings, which sat among the freed
 * pages, so 16 MiB failed though 1 and 4 MiB worked; procnto serves it
 * (measured). Prints one "ok"/"FAIL" line per check and
 * "contigtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syspage.h>
#include <process.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("contigtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static void *contig(size_t len)
{
    return mmap(NULL, len, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANON | MAP_PHYS, NOFD, 0);
}

/* use most of memory, in 16 MiB pieces, and free it all; returns the MiB used */
static int hog(void)
{
    enum { PIECE = 16 << 20, MAX = 128 };
    void *pieces[MAX];
    int n = 0;
    for (; n < MAX; n++) {
        pieces[n] = mmap(NULL, PIECE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, NOFD, 0);
        if (pieces[n] == MAP_FAILED) {
            break;
        }
        memset(pieces[n], 0x5a, 4096); /* superlocked anyway */
    }
    /* every other piece first, then the rest: the free memory is in
     * pieces before it is whole again */
    for (int i = 0; i < n; i += 2) munmap(pieces[i], PIECE);
    for (int i = 1; i < n; i += 2) munmap(pieces[i], PIECE);
    return n * 16;
}

int main(int argc, char **argv)
{
    if (argc > 1 && strcmp(argv[1], "hog") == 0) {
        printf("contigtest: the child used and freed %d MiB\n", hog());
        return 0;
    }
    char what[120];
    /* a child uses and frees most of memory, and ends */
    int st = 0;
    pid_t pid = spawnl(P_WAIT, "/proc/boot/contigtest", "contigtest", "hog", NULL);
    check("a child used most of memory, freed it and ended", pid == 0 && st == 0);
    static const size_t sizes[] = { 1 << 20, 4 << 20, 16 << 20 };
    for (unsigned i = 0; i < sizeof sizes / sizeof *sizes; i++) {
        errno = 0;
        void *p = contig(sizes[i]);
        snprintf(what, sizeof what, "mmap(MAP_PHYS | MAP_ANON) of %zu MiB -> %s errno %d", sizes[i] >> 20,
                 p == MAP_FAILED ? "failed" : "ok", errno);
        check(what, p != MAP_FAILED);
        if (p != MAP_FAILED) {
            off64_t pa = 0;
            size_t clen = 0;
            int r = mem_offset64(p, NOFD, sizes[i], &pa, &clen);
            snprintf(what, sizeof what, "  it is contiguous (%zu bytes) and below 4 GiB", clen);
            check(what, r == 0 && clen >= sizes[i] && (uint64_t)pa + sizes[i] <= (1ULL << 32));
            memset(p, 0xa5, sizes[i]);
            munmap(p, sizes[i]);
        }
    }
    /* this process uses and frees most of memory, and goes on */
    int mib = hog();
    errno = 0;
    void *p = contig(16 << 20);
    snprintf(what, sizeof what, "after using and freeing %s MiB itself: 16 MiB contiguous -> %s errno %d",
             mib > 256 ? "over 256" : "under 256", p == MAP_FAILED ? "failed" : "ok", errno);
    check(what, p != MAP_FAILED);
    if (p != MAP_FAILED) munmap(p, 16 << 20);
    int fd = shm_open("/contigtest", O_RDWR | O_CREAT, 0600);
    errno = 0;
    int r = shm_ctl(fd, SHMCTL_ANON | SHMCTL_PHYS, 0, 16 << 20);
    snprintf(what, sizeof what, "shm_ctl(SHMCTL_ANON | SHMCTL_PHYS) of 16 MiB -> %d errno %d", r, errno);
    check(what, r == 0);
    close(fd);
    shm_unlink("/contigtest");
    /* again and again: freed contiguous memory is contiguous again */
    int ok = 1;
    for (int i = 0; i < 64 && ok; i++) {
        void *p = contig(4 << 20);
        ok = p != MAP_FAILED;
        if (ok) munmap(p, 4 << 20);
    }
    check("64 rounds of 4 MiB contiguous, mapped and freed", ok);
    printf("contigtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

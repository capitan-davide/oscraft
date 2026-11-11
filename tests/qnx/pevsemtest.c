/* Process-manager events delivered as SIGEV_SEM (a named semaphore, the
 * only kind QNX 8's SIGEV_SEM_INIT() takes) while the path manager and
 * the memory manager work (review PATHMEM-2): the PATHSPACE event of a
 * name_attach() and of a name_detach() (an _IO_CLOSE of the attached
 * connection), and the CONTIG_ALLOC_FAIL event of a shm_ctl() and of an
 * mmap() of contiguous memory that can't be had. The kernel used to
 * deliver the detach's and shm_ctl()'s events while holding the
 * open-file table that finding the semaphore locks again: a hang.
 * procnto posts the semaphore once per event (measured); the same
 * events as pulses carry the name's hash (ELF hash of the absolute
 * path, top bit set) or 0. Prints one "ok"/"FAIL" line per check and
 * "pevsemtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <semaphore.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/dispatch.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <unistd.h>

static int fails;
static int chid, coid;

static void check(const char *what, int ok)
{
    printf("pevsemtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

/* the PATHSPACE event's value for a path */
static unsigned pathspace_hash(const char *path)
{
    unsigned hash = 0;
    for (; *path; path++) {
        hash = (hash << 4) + (unsigned char)*path;
        unsigned x = hash & 0xf0000000u;
        if (x) {
            hash ^= x >> 24;
            hash &= ~x;
        }
    }
    return hash | 0x80000000u;
}

/* whether a pulse with `value` arrives within 300 ms (others may come:
 * services attaching names as they start) */
static int pulse_with(unsigned value)
{
    struct _pulse pulse;
    for (;;) {
        uint64_t to = 300000000ULL;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, NULL, &to, NULL);
        if (MsgReceivePulse(chid, &pulse, sizeof pulse, NULL) == -1) {
            return 0;
        }
        if ((unsigned)pulse.value.sival_int == value) {
            return 1;
        }
    }
}

static int sem_value(sem_t *sem)
{
    int v = -1;
    sem_getvalue(sem, &v);
    return v;
}

static void fail_contig(const char *how, sem_t *sem)
{
    char what[96];
    int fd = shm_open("/pevsemtest", O_RDWR | O_CREAT, 0600);
    errno = 0;
    int r = shm_ctl(fd, SHMCTL_ANON | SHMCTL_PHYS, 0, 1ULL << 40);
    snprintf(what, sizeof what, "%s: shm_ctl(ANON | PHYS) of 1 TiB is ENOMEM", how);
    check(what, r == -1 && errno == ENOMEM);
    close(fd);
    shm_unlink("/pevsemtest");
    snprintf(what, sizeof what, "%s: it raised CONTIG_ALLOC_FAIL", how);
    check(what, sem ? sem_value(sem) == 3 : pulse_with(0));
    errno = 0;
    void *p = mmap(NULL, 1ULL << 40, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON | MAP_PHYS, NOFD, 0);
    snprintf(what, sizeof what, "%s: mmap(MAP_PHYS | MAP_ANON) of 1 TiB is ENOMEM", how);
    check(what, p == MAP_FAILED && errno == ENOMEM);
    snprintf(what, sizeof what, "%s: it raised CONTIG_ALLOC_FAIL", how);
    check(what, sem ? sem_value(sem) == 4 : pulse_with(0));
}

int main(void)
{
    chid = ChannelCreate(0);
    coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);
    unsigned hash = pathspace_hash("/dev/name/local/pevsemtest");
    struct sigevent ev;

    /* as pulses first: the events and their values */
    SIGEV_PULSE_INIT(&ev, coid, 10, 7, 0);
    SIGEV_MAKE_UPDATEABLE(&ev);
    check("MsgRegisterEvent() of the pulse", MsgRegisterEvent(&ev, SYSMGR_COID) == 0);
    check("procmgr_event_notify(PATHSPACE | CONTIG_ALLOC_FAIL)",
          procmgr_event_notify(PROCMGR_EVENT_PATHSPACE | PROCMGR_EVENT_CONTIG_ALLOC_FAIL, &ev) == 0);
    name_attach_t *att = name_attach(NULL, "pevsemtest", 0);
    check("name_attach()", att != NULL);
    check("pulse: the attach raised PATHSPACE with the name's hash", pulse_with(hash));
    check("name_detach()", name_detach(att, 0) == 0);
    check("pulse: the detach raised PATHSPACE with the name's hash", pulse_with(hash));
    fail_contig("pulse", NULL);

    /* then as a named semaphore: the deliveries that re-lock the table */
    sem_t *sem = sem_open("/pevsemtest", O_CREAT, 0600, 0);
    check("sem_open()", sem != SEM_FAILED);
    SIGEV_SEM_INIT(&ev, sem);
    check("MsgRegisterEvent() of the semaphore", MsgRegisterEvent(&ev, SYSMGR_COID) == 0);
    check("procmgr_event_notify(PATHSPACE | CONTIG_ALLOC_FAIL) with SIGEV_SEM",
          procmgr_event_notify(PROCMGR_EVENT_PATHSPACE | PROCMGR_EVENT_CONTIG_ALLOC_FAIL, &ev) == 0);
    att = name_attach(NULL, "pevsemtest", 0);
    check("sem: the attach posted it once", att != NULL && sem_value(sem) == 1);
    check("name_detach() with the semaphore registered returns", name_detach(att, 0) == 0);
    check("sem: the detach posted it once", sem_value(sem) == 2);
    fail_contig("sem", sem);
    sem_close(sem);
    sem_unlink("/pevsemtest");
    printf("pevsemtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

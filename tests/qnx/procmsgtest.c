/* procnto's messages at their edges (as measured on procnto): too short
 * for their type (EBADMSG), subtypes it doesn't know, pid 0 where it
 * names no process, and what getpagesizes(), _PROC_GETSETID,
 * _PROC_EVENT_NOTIFY_ADD, _PROC_GET_ALL_CHILD_PID, _SYS_TRACE,
 * _MEM_SHMEM_HANDLE and _MEM_CTRL answer there; 64-bit pids are 1 above
 * the pid (procnto's own is 1); watched values (procmgr_value_*()); the
 * memory manager's address space queries (_MEM_AS_INFO, _MEM_MAP_INFO,
 * _MEM_OFFSET) and the messages it takes or refuses as they are.
 * Prints one "ok"/"FAIL" line per check and "procmsgtest: passed". */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/debug.h>
#include <sys/memmsg.h>
#include <sys/mman.h>
#include <sys/neutrino.h>
#include <sys/procmgr.h>
#include <sys/procfs.h>
#include <sys/procmsg.h>
#include <sys/sysmsg.h>
#include <sys/wait.h>
#include <unistd.h>

static int fails;

static void check(const char *what, int ok)
{
    printf("procmsgtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fflush(stdout);
    fails += !ok;
}

static unsigned char rbuf[8192];
static int rlen;

/* Send `len` bytes of `m` to procnto; the status, or -errno. The reply's
 * length (bytes written) in rlen. */
static long send(const void *m, unsigned len, unsigned room)
{
    memset(rbuf, 0xee, sizeof rbuf);
    long s = MsgSend(SYSMGR_COID, m, len, rbuf, room);
    rlen = 0;
    for (int i = sizeof rbuf - 1; i >= 0; i--) {
        if (rbuf[i] != 0xee) {
            rlen = i + 1;
            break;
        }
    }
    return s == -1 ? -errno : s;
}

/* A zeroed message of type `type` and subtype `sub`. */
static long zeroed(unsigned type, unsigned sub, unsigned len)
{
    unsigned char m[128];
    memset(m, 0, sizeof m);
    m[0] = type & 0xff;
    m[1] = type >> 8;
    m[2] = sub;
    return send(m, len, sizeof rbuf);
}

static void short_messages(void)
{
    check("_SYS_CONF of 4 bytes EBADMSG", zeroed(_SYS_CONF, 0, 4) == -EBADMSG);
    check("_SYS_CMD of 4 bytes EBADMSG (not a reboot)", zeroed(_SYS_CMD, _SYS_CMD_REBOOT, 4) == -EBADMSG);
    check("_PROC_GETSETID of 16 bytes EBADMSG", zeroed(_PROC_GETSETID, 0, 16) == -EBADMSG);
    check("_MEM_MAP of 64 bytes EBADMSG", zeroed(_MEM_MAP, 0, 64) == -EBADMSG);
    check("_MEM_CTRL of 20 bytes EBADMSG", zeroed(_MEM_CTRL, 0, 20) == -EBADMSG);
    check("a type procnto doesn't handle: ENOSYS at any length", zeroed(_SYS_LOG, 0, 2) == -ENOSYS);
    check("_PROC_TIMER_TOLERANCE subtype 2 ENOSYS", zeroed(_PROC_TIMER_TOLERANCE, 2, 16) == -ENOSYS);
    check("_PROC_RESOURCE limits with a count of 0 ENOTSUP",
          zeroed(_PROC_RESOURCE, _PROC_RESOURCE_GETLIMIT, 64) == -ENOTSUP);
    check("_PROC_GETPID64 of pid 0 ESRCH", zeroed(_PROC_GETPID64, 0, 8) == -ESRCH);
    struct _proc_getpid64 g64 = { _PROC_GETPID64, 0, getpid() };
    uint64_t me64 = 0;
    long r64 = send(&g64, sizeof g64, 8);
    memcpy(&me64, rbuf, 8);
    g64.pid = 1;
    send(&g64, sizeof g64, 8);
    uint64_t one64;
    memcpy(&one64, rbuf, 8);
    check("_PROC_GETPID64: 1 above the pid, procnto's 1",
          r64 == 0 && me64 == (1ULL << 32 | (unsigned)getpid()) && one64 == 1);
    check("_PROC_GETNAME of pid 0 ESRCH", zeroed(_PROC_GETNAME, 0, 16) == -ESRCH);
    check("_PROC_DEBUG registers of thread 0 EINVAL",
          zeroed(_PROC_DEBUG, _PROC_DEBUG_GET_GENERAL_REGS, 16) == -EINVAL);
    check("_MEM_SHMEM_HANDLE subtype 3 EBADMSG", zeroed(_MEM_SHMEM_HANDLE, 3, 40) == -EBADMSG);
    check("_MEM_SHMEM_HANDLE for pid 0 ESRCH", zeroed(_MEM_SHMEM_HANDLE, 0, 40) == -ESRCH);
    check("_MEM_PEER of nothing EINVAL", zeroed(_MEM_PEER, 0, 64) == -EINVAL);
    check("_MEM_OFFSET of an unmapped address EACCES, whatever the subtype",
          zeroed(_MEM_OFFSET, _MEM_OFFSET_FD, 24) == -EACCES && zeroed(_MEM_OFFSET, _MEM_OFFSET_PT, 24) == -EACCES);
}

static long getid(int sub, int ngroups)
{
    unsigned char m[20];
    memset(m, 0, sizeof m);
    m[0] = _PROC_GETSETID;
    m[2] = sub;
    *(int *)(m + 8) = -1;
    *(int *)(m + 12) = -1;
    *(int *)(m + 16) = ngroups;
    return send(m, sizeof m, sizeof rbuf);
}

static int add_event(void)
{
    unsigned char m[8 + sizeof(struct sigevent)];
    memset(m, 0, sizeof m);
    m[0] = _PROC_EVENT;
    m[2] = _PROC_EVENT_NOTIFY_ADD;
    *(unsigned *)(m + 4) = PROCMGR_EVENT_PROCESS_CREATE;
    struct sigevent ev;
    SIGEV_NONE_INIT(&ev);
    memcpy(m + 8, &ev, sizeof ev);
    return (int)send(m, sizeof m, sizeof rbuf);
}

static void answers(void)
{
    size_t ps[4];
    int n = getpagesizes(ps, 4);
    check("getpagesizes(): one, 4 KB", n == 1 && ps[0] == 4096 && getpagesizes(NULL, 0) == 1);

    gid_t g[2] = { 5, 6 };
    setgroups(2, g);
    long s = getid(0, 0);
    printf("procmsgtest:   GETID: %ld, %d bytes\n", s, rlen);
    check("_PROC_ID_GETID: every group, whatever ngroups asks", s == 0 && rlen == 56 + 8);
    s = getid(8, 8);
    check("_PROC_ID_GETID_NO_CRED: no credentials", s == 0 && rlen == 28);
    check("setuid(-1) EINVAL", getid(1, 0) == -EINVAL && getuid() == 0);
    s = getid(5, 0); /* setreuid(-1, -1) */
    check("a change has no reply", s == 0 && rlen == 0);

    int a = add_event(), b = add_event();
    pid_t c = fork();
    if (c == 0) {
        _exit(add_event() == 1 ? 0 : 1);
    }
    int st;
    waitpid(c, &st, 0);
    check("_PROC_EVENT_NOTIFY_ADD IDs count per process from 1",
          a == 1 && b == 2 && WIFEXITED(st) && WEXITSTATUS(st) == 0);

    pid_t k1 = fork();
    if (k1 == 0) {
        pause();
        _exit(0);
    }
    pid_t k2 = fork();
    if (k2 == 0) {
        pause();
        _exit(0);
    }
    unsigned char m[16];
    memset(m, 0, sizeof m);
    m[0] = _PROC_GET_ALL_CHILD_PID;
    s = send(m, sizeof m, sizeof rbuf);
    uint64_t pids[2];
    memcpy(pids, rbuf, sizeof pids);
    uint64_t p1 = 1ULL << 32 | (unsigned)k1, p2 = 1ULL << 32 | (unsigned)k2;
    check("_PROC_GET_ALL_CHILD_PID: the children's 64-bit pids (1 above the pid)",
          s == 16 && rlen == 16 && ((pids[0] == p1 && pids[1] == p2) || (pids[0] == p2 && pids[1] == p1)));
    s = send(m, sizeof m, 8);
    check("the status is their size, whatever the buffer holds", s == 16 && rlen == 8);
    *(uint64_t *)(m + 8) = 99999;
    check("no such process ESRCH", send(m, sizeof m, sizeof rbuf) == -ESRCH);
    kill(k1, SIGKILL);
    kill(k2, SIGKILL);
    waitpid(k1, NULL, 0);
    waitpid(k2, NULL, 0);

    check("_SYS_TRACE: no buffers EINVAL, freeing others' EPERM, detach, subtype 3 ENOSYS",
          zeroed(_SYS_TRACE, _SYS_TRACE_ALLOCBUF, 8) == -EINVAL &&
              zeroed(_SYS_TRACE, _SYS_TRACE_DEALLOCBUF, 8) == -EPERM && zeroed(_SYS_TRACE, _SYS_TRACE_DETACH, 8) == 0 &&
              zeroed(_SYS_TRACE, 3, 8) == -ENOSYS);
}

#define E(x) ((x) == 0 ? 0 : errno)

static void ranges(void)
{
    char *m = mmap(NULL, 2 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    check("mprotect(), msync(), mlock() of no page ENOMEM; of a byte's page 0",
          E(mprotect(m, 0, PROT_READ | PROT_WRITE)) == ENOMEM && E(msync(m, 0, MS_SYNC)) == ENOMEM &&
              E(mlock(m, 0)) == ENOMEM && E(mprotect(m + 1, 0, PROT_READ | PROT_WRITE)) == 0 &&
              E(mlock(m + 1, 0)) == 0 && E(munlock(m, 0)) == 0);
    check("at address 0: unmapped (ENOMEM), munmap() 0",
          E(mprotect(NULL, 4096, PROT_READ)) == ENOMEM && E(msync(NULL, 4096, MS_SYNC)) == ENOMEM &&
              E(mlock(NULL, 4096)) == ENOMEM && E(munmap(NULL, 4096)) == 0);
    munmap(m, 2 * 4096);
}

static debug_aspace_t as_info(int pid)
{
    struct _mem_as_info m = { _MEM_AS_INFO, 0, pid };
    debug_aspace_t a;
    memset(&a, 0, sizeof a);
    if (MsgSend(MEMMGR_COID, &m, sizeof m, &a, sizeof a) == -1) {
        a.as_size = 0;
    }
    return a;
}

/* _MEM_MAP_INFO of subtype `sub` over [start, end]: the status. */
static long map_info(int sub, uint64_t start, uint64_t end)
{
    mem_map_info_i_t m;
    memset(&m, 0, sizeof m);
    m.type = _MEM_MAP_INFO;
    m.subtype = sub;
    m.startaddr = start;
    m.endaddr = end;
    return send(&m, sizeof m, sizeof rbuf);
}

static void memory(void)
{
    debug_aspace_t a0 = as_info(0), self = as_info(getpid());
    printf("procmsgtest:   AS_INFO: size %#llx used %#llx count %llu private %#llx shared %#llx rss %#llx\n",
           (unsigned long long)a0.as_size, (unsigned long long)a0.as_used, (unsigned long long)a0.map_count,
           (unsigned long long)a0.map_private, (unsigned long long)a0.map_shared, (unsigned long long)a0.rss);
    check("_MEM_AS_INFO: pid 0 is the caller; used = private + shared = resident",
          a0.as_size == 0x7fffffffffULL && a0.as_used == self.as_used && a0.map_count == self.map_count &&
              a0.as_used == a0.map_private + a0.map_shared && a0.rss == a0.as_used);
    check("procnto's address space has no end", as_info(1).as_size == ~0ULL);
    char *priv = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    debug_aspace_t a1 = as_info(0);
    int fd = shm_open("/procmsgtest", O_RDWR | O_CREAT, 0600);
    ftruncate(fd, 1 << 20);
    char *sh = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    debug_aspace_t a2 = as_info(0);
    check("a private mapping counts as private, a shared one as shared",
          a1.map_private - a0.map_private == 1 << 20 && a1.map_shared == a0.map_shared &&
              a2.map_shared - a1.map_shared == 1 << 20 && a2.map_count == a1.map_count + 1);

    uintptr_t p = (uintptr_t)priv;
    long n = map_info(_MEM_MAP_INFO_PAGETABLE, p, p + 0x3000);
    procfs_mapinfo *e = (void *)rbuf;
    (void)sh;
    check("_MEM_MAP_INFO_PAGETABLE: each page, the last one included, with its frame",
          n == 4 * (long)sizeof *e && e[0].vaddr == p && e[0].size == 4096 && e[3].vaddr == p + 0x3000 &&
              e[0].paddr != 0);
    uint64_t paddr0 = e[0].paddr;
    n = map_info(_MEM_MAP_INFO_PHYSICAL, p, p + 0x3000);
    check("_MEM_MAP_INFO_PHYSICAL: contiguous runs from the region's start",
          n >= (long)sizeof *e && e[0].vaddr == p && e[0].paddr == paddr0);
    n = map_info(_MEM_MAP_INFO_OBJECT_PATH, 0, 0);
    n = map_info(_MEM_MAP_INFO_VIRTUAL, 0, ~0ULL);
    uint64_t lowest = e[0].vaddr, top = 0;
    for (long i = 0; i < n / (long)sizeof *e && i < rlen / (long)sizeof *e; i++) {
        if (e[i].vaddr + e[i].size > top) {
            top = e[i].vaddr + e[i].size;
        }
    }
    long above = top >= 0x8000000000ULL ? -ENXIO : map_info(_MEM_MAP_INFO_OBJECT_PATH, top, 0);
    printf("procmsgtest:   regions %ld, top %#llx: %ld\n", n / (long)sizeof *e, (unsigned long long)top, above);
    n = map_info(_MEM_MAP_INFO_OBJECT_PATH, 0, 0);
    check("_MEM_MAP_INFO_OBJECT_PATH where nothing is mapped: the next region above",
          n > 8 && *(uint64_t *)rbuf == lowest);
    check("none above ENXIO", above == -ENXIO);
    struct _mem_offset off = { _MEM_OFFSET, _MEM_OFFSET_FD, 0, p, 4096 };
    long s1 = send(&off, sizeof off, sizeof rbuf);
    struct _mem_offset_reply o1 = *(struct _mem_offset_reply *)rbuf;
    off.subtype = _MEM_OFFSET_PT;
    long s2 = send(&off, sizeof off, sizeof rbuf);
    struct _mem_offset_reply o2 = *(struct _mem_offset_reply *)rbuf;
    check("_MEM_OFFSET_FD with a descriptor, and _PT: the physical address",
          s1 == 0 && s2 == 0 && o1.offset == paddr0 && o2.offset == paddr0 && o1.fd == -1 && o1.size == 4096);
    struct _mem_obj_info oi = { _MEM_OBJ_INFO, 0, fd, 0, 1 << 20 };
    check("_MEM_OBJ_INFO EBADF, even for shared memory", send(&oi, sizeof oi, sizeof rbuf) == -EBADF);
    check("_MEM_DEBUG_INFO and _64 taken", zeroed(_MEM_DEBUG_INFO, 0, 48) == 0 && zeroed(_MEM_DEBUG_INFO64, 0, 64) == 0);
    struct _mem_pmem_add pa = { _MEM_PMEM_ADD, 0, 0, 0x100000000ULL, 0x1000 };
    check("_MEM_PMEM_ADD EINVAL", send(&pa, sizeof pa, sizeof rbuf) == -EINVAL);
    munmap(priv, 1 << 20);
    munmap(sh, 1 << 20);
    close(fd);
    shm_unlink("/procmsgtest");
}

static int vchid, vcoid;

static int value_add(unsigned type, int sub, uint64_t point, int code)
{
    struct sigevent ev;
    SIGEV_PULSE_INIT(&ev, vcoid, SIGEV_PULSE_PRIO_INHERIT, code, 0);
    MsgRegisterEvent(&ev, SYSMGR_COID);
    int h = procmgr_value_notify_add(type, sub, point, &ev);
    return h == -1 ? -errno : h;
}

/* The codes of the pulses that came within 50 ms, as a bit mask. */
static unsigned value_pulses(void)
{
    unsigned got = 0;
    struct _pulse p;
    struct sigevent to;
    SIGEV_UNBLOCK_INIT(&to);
    for (;;) {
        uint64_t ns = 50000000;
        TimerTimeout(CLOCK_MONOTONIC, _NTO_TIMEOUT_RECEIVE, &to, &ns, NULL);
        if (MsgReceivePulse(vchid, &p, sizeof p, NULL) == -1) {
            return got;
        }
        got |= 1u << p.code;
    }
}

static void values(void)
{
    vchid = ChannelCreate(_NTO_CHF_PRIVATE);
    vcoid = ConnectAttach(0, 0, vchid, _NTO_SIDE_CHANNEL, 0);
    struct sigevent none = { .sigev_notify = SIGEV_NONE };
    int e = procmgr_event_notify_add(PROCMGR_EVENT_PROCESS_CREATE, &none);
    int hf = value_add(PROCMGR_VALUE_FREE_MEM, 0, 0, 1);
    int hp = value_add(PROCMGR_VALUE_PRIVATE_MEM, getpid(), 0, 2);
    printf("procmsgtest:   IDs: event %d, free %d, private %d\n", e, hf, hp);
    check("procmgr_value_notify_add(): IDs follow the process's event IDs", hf == e + 1 && hp == e + 2);
    check("process 0 or none ESRCH, a bad type EINVAL",
          value_add(PROCMGR_VALUE_PRIVATE_MEM, 0, 0, 3) == -ESRCH &&
              value_add(PROCMGR_VALUE_PRIVATE_MEM, 99999, 0, 3) == -ESRCH && value_add(7, 0, 0, 3) == -EINVAL);
    errno = 0;
    check("procmgr_value_current() of no such ID ESRCH", procmgr_value_current(12345) == (uint64_t)-1 && errno == ESRCH);
    uint64_t f0 = procmgr_value_current(hf), p0 = procmgr_value_current(hp);
    char *m = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    uint64_t f1 = procmgr_value_current(hf), p1 = procmgr_value_current(hp);
    munmap(m, 1 << 20);
    uint64_t p2 = procmgr_value_current(hp);
    check("free memory: bytes, down by a 1 MB mapping", f0 > (64u << 20) && f0 - f1 >= (1u << 20));
    check("private memory: bytes, exactly 1 MB more while it's mapped", p1 - p0 == (1u << 20) && p2 == p0);
    int up = value_add(PROCMGR_VALUE_PRIVATE_MEM | PROCMGR_VALUE_TRIGGER_UP, getpid(), p0 + (512 << 10), 10);
    int down = value_add(PROCMGR_VALUE_PRIVATE_MEM | PROCMGR_VALUE_TRIGGER_DOWN, getpid(), p0 + (512 << 10), 11);
    int now = value_add(PROCMGR_VALUE_PRIVATE_MEM | PROCMGR_VALUE_TRIGGER_UP, getpid(), p0, 12);
    unsigned got0 = value_pulses();
    m = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    unsigned got1 = value_pulses();
    munmap(m, 1 << 20);
    unsigned got2 = value_pulses();
    m = mmap(NULL, 1 << 20, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
    unsigned got3 = value_pulses();
    munmap(m, 1 << 20);
    value_pulses();
    printf("procmsgtest:   triggers %d %d %d: at once %#x, map %#x, unmap %#x, map %#x\n", up, down, now, got0, got1, got2,
           got3);
    check("a trigger already reached (up: at or above, down: at or below) is delivered at once",
          got0 == (1u << 11 | 1u << 12));
    check("up and down: each crossing", got1 == 1u << 10 && got2 == 1u << 11 && got3 == 1u << 10);
    errno = 0;
    check("deleted with procmgr_event_notify_delete()",
          procmgr_event_notify_delete(up) == 0 && procmgr_value_current(up) == (uint64_t)-1 && errno == ESRCH);
}

int main(void)
{
    short_messages();
    answers();
    ranges();
    values();
    memory();
    printf("procmsgtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

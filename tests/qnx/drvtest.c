/* Kernel calls for drivers: CacheControl() (and msync()'s cache-only
 * flags, which libc sends as CacheControl()), MsgVerifyEvent() from a
 * client (rcvid 0) and from a server, MsgPause() (not supported in QNX 8),
 * InterruptQuery(), and InterruptUnblock() getting an
 * interrupt thread out of InterruptWait(), InterruptAttach()'s errors (no
 * handler functions in QNX 8, synthetic vectors), InterruptCtl() (no
 * commands in procnto: ENOSYS). The interrupt thread attaches
 * with _NTO_INTR_FLAGS_NO_UNMASK to a vector no driver uses, so the
 * hardware interrupt stays masked. Prints one "ok"/"FAIL" line per check
 * and "drvtest: passed" at the end. Needs root (interrupts). */
#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/interrupts.h>
#include <sys/mman.h>
#include <sys/syspage.h>
#include <sys/neutrino.h>
#include <unistd.h>

/* In libc and documented under <sys/neutrino.h>, which (SDP 8.0.5)
 * doesn't declare it. */
extern int CacheControl(void *addr, size_t len, unsigned flags);

static int fails;

static void check(const char *what, int ok)
{
    printf("drvtest: %s %s\n", ok ? "ok" : "FAIL", what);
    fails += !ok;
}

static void cache(void)
{
    size_t len = 3 * 4096 + 100;
    char *buf = malloc(len);
    memset(buf, 0x5a, len);
    check("CacheControl(MS_SYNC)", CacheControl(buf + 1, len - 1, MS_SYNC) == 0);
    check("CacheControl(MS_INVALIDATE)", CacheControl(buf, len, MS_INVALIDATE) == 0 && buf[len - 1] == 0x5a);
    check("CacheControl(MS_INVALIDATE_ICACHE)", CacheControl(buf, len, MS_INVALIDATE_ICACHE) == 0);
    check("CacheControl() of unmapped memory: skipped, no error",
          CacheControl((void *)0x1000, 4096, MS_SYNC) == 0);
    check("msync(MS_CACHE_ONLY | MS_SYNC)", msync(buf, len, MS_CACHE_ONLY | MS_SYNC) == 0);
    free(buf);
}

static int chid, srv_coid;
static volatile int server_fails = -1;

static int expect(int r, int err, int want)
{
    return want == EOK ? r == EOK : r == -1 && err == want;
}

/* A server may deliver (or verify) only registered events; any other is
 * EACCES (QNX 8 without procnto -U). */
static void *server(void *arg)
{
    struct sigevent reg; /* the client's registered event */
    struct sigevent ev;
    int bad = 0;
    (void)arg;
    rcvid_t rcvid = MsgReceive(chid, &reg, sizeof reg, NULL);
#define SRV(what, call, want)                                                  \
    do {                                                                       \
        errno = 0;                                                             \
        int r_ = (call);                                                       \
        int ok_ = expect(r_, errno, want);                                     \
        printf("drvtest: %s server: %s (%d, errno %d)\n", ok_ ? "ok" : "FAIL", what, r_, errno); \
        bad += !ok_;                                                           \
    } while (0)
    SIGEV_PULSE_INIT(&ev, srv_coid, -1, _PULSE_CODE_MINAVAIL, 0);
    SRV("MsgVerifyEvent(unregistered pulse): EACCES", MsgVerifyEvent(rcvid, &ev), EACCES);
    SRV("MsgDeliverEvent(unregistered pulse): EACCES", MsgDeliverEvent(rcvid, &ev), EACCES);
    SRV("MsgVerifyEvent(registered)", MsgVerifyEvent(rcvid, &reg), EOK);
    SRV("MsgDeliverEvent(registered)", MsgDeliverEvent(rcvid, &reg), EOK);
    SIGEV_SIGNAL_INIT(&ev, SIGUSR2);
    SRV("MsgVerifyEvent(unregistered signal): EACCES", MsgVerifyEvent(rcvid, &ev), EACCES);
    SIGEV_NONE_INIT(&ev);
    SRV("MsgVerifyEvent(unregistered none): EACCES", MsgVerifyEvent(rcvid, &ev), EACCES);
    SIGEV_UNBLOCK_INIT(&ev);
    SRV("MsgVerifyEvent(unregistered unblock): EACCES", MsgVerifyEvent(rcvid, &ev), EACCES);
    server_fails = bad;
    MsgReply(rcvid, 0, NULL, 0);
    return NULL;
}

static void verify(void)
{
    struct sigevent ev;
    chid = ChannelCreate(0);
    srv_coid = ConnectAttach(0, 0, chid, _NTO_SIDE_CHANNEL, 0);

    SIGEV_PULSE_INIT(&ev, srv_coid, -1, _PULSE_CODE_MINAVAIL, 7);
    check("MsgVerifyEvent(0, pulse)", MsgVerifyEvent(0, &ev) == EOK);
    SIGEV_PULSE_INIT(&ev, 1000, -1, _PULSE_CODE_MINAVAIL, 7);
    errno = 0;
    check("MsgVerifyEvent(0, pulse on no connection): EBADF", MsgVerifyEvent(0, &ev) == -1 && errno == EBADF);
    SIGEV_SIGNAL_INIT(&ev, SIGUSR1);
    check("MsgVerifyEvent(0, signal)", MsgVerifyEvent(0, &ev) == EOK);
    SIGEV_SIGNAL_INIT(&ev, 0);
    errno = 0;
    check("MsgVerifyEvent(0, signal 0): EINVAL", MsgVerifyEvent(0, &ev) == -1 && errno == EINVAL);
    errno = 0;
    SIGEV_SIGNAL_INIT(&ev, SIGUSR1);
    check("MsgVerifyEvent(no such rcvid): ESRCH", MsgVerifyEvent(0x7ffff0, &ev) == -1 && errno == ESRCH);

    struct sigevent reg;
    SIGEV_PULSE_INIT(&reg, srv_coid, -1, _PULSE_CODE_MINAVAIL, 9);
    check("MsgRegisterEvent()", MsgRegisterEvent(&reg, srv_coid) == 0);
    pthread_t t;
    pthread_create(&t, NULL, server, NULL);
    MsgSend(srv_coid, &reg, sizeof reg, NULL, 0);
    pthread_join(t, NULL);
    fails += server_fails != 0;

    SIGEV_PULSE_INIT(&ev, srv_coid, -1, _PULSE_CODE_MINAVAIL, 7);
    check("MsgDeliverEvent(0, unregistered pulse) to itself", MsgDeliverEvent(0, &ev) == 0);
    errno = 0;
    check("MsgPause(): ENOSYS", MsgPause(1, 0) == -1 && errno == ENOSYS);
}

static int intr_id = -1, attached_vector;
static volatile pthread_t ist_tid;
static volatile int ist_state; /* 1 waiting, 2 woken */

static void *ist(void *arg)
{
    /* Unused SPIs of the BCM2711's GIC, the first that attaches. */
    static const int vectors[] = { 220, 210, 200 };
    (void)arg;
    for (unsigned i = 0; i < sizeof vectors / sizeof vectors[0] && intr_id == -1; i++) {
        intr_id = InterruptAttachThread(vectors[i], _NTO_INTR_FLAGS_NO_UNMASK);
        attached_vector = vectors[i];
    }
    if (intr_id == -1) {
        ist_state = -1;
        return NULL;
    }
    ist_tid = pthread_self();
    ist_state = 1;
    int r = InterruptWait(0, NULL);
    ist_state = r == 0 ? 2 : -2;
    InterruptDetach(intr_id);
    return NULL;
}

static void query(void)
{
    struct interrupt_query_data d;
    memset(&d, 0xee, sizeof d);
    int r = InterruptQuery(_NTO_INTR_QUERY_HANDLER, intr_id, 0, &d);
    check("InterruptQuery(HANDLER)",
          r == 0 && d.type == _NTO_INTR_QUERY_HANDLER && d.vector == (unsigned)attached_vector
              && d.deliver_count == 0 && d.data.handler.ist_pid == getpid()
              && d.data.handler.ist_tid == ist_tid && d.data.handler.ist_runmask == ~0ULL
              && d.data.handler.attach_id == intr_id
              && d.data.handler.attach_flags == (_NTO_INTR_FLAGS_NO_UNMASK | _NTO_INTR_FLAGS_TRK_MSK)
              && d.data.handler.mask_count == 1 && d.data.handler.event.sigev_notify == 0);
    if (d.data.handler.attach_id != intr_id || d.data.handler.ist_tid != ist_tid || d.data.handler.attach_flags != 0xa) {
        printf("drvtest:   got %d: type %u vector %u count %llu pid %d tid %d (%d) runmask %#llx id %d flags %#x "
               "mask %u notify %d\n", r, d.type, d.vector, (unsigned long long)d.deliver_count,
               d.data.handler.ist_pid, d.data.handler.ist_tid, (int)ist_tid,
               (unsigned long long)d.data.handler.ist_runmask, d.data.handler.attach_id,
               d.data.handler.attach_flags, d.data.handler.mask_count, d.data.handler.event.sigev_notify);
    }
    memset(&d, 0xee, sizeof d);
    r = InterruptQuery(_NTO_INTR_QUERY_SOURCE, attached_vector, 0, &d);
    check("InterruptQuery(SOURCE), masked",
          r == 0 && d.type == _NTO_INTR_QUERY_SOURCE && d.vector == (unsigned)attached_vector
              && d.data.source.num_handlers == 1 && d.data.source.mask_count == 1);
    InterruptUnmask(attached_vector, intr_id);
    r = InterruptQuery(_NTO_INTR_QUERY_SOURCE, attached_vector, 0, &d);
    check("InterruptQuery(SOURCE), unmasked", r == 0 && d.data.source.mask_count == 0);
    r = InterruptQuery(_NTO_INTR_QUERY_HANDLER, intr_id, 0, &d);
    check("InterruptQuery(HANDLER), unmasked", r == 0 && d.data.handler.mask_count == 0);
    InterruptMask(attached_vector, intr_id);

    errno = 0;
    check("InterruptQuery(HANDLER, no such id): ESRCH",
          InterruptQuery(_NTO_INTR_QUERY_HANDLER, 99999, 0, &d) == -1 && errno == ESRCH);
    errno = 0;
    check("InterruptQuery(SOURCE, no such vector): EINVAL",
          InterruptQuery(_NTO_INTR_QUERY_SOURCE, 0x7fffff, 0, &d) == -1 && errno == EINVAL);
    errno = 0;
    check("InterruptQuery(type 7): EINVAL", InterruptQuery(7, intr_id, 0, &d) == -1 && errno == EINVAL);
    int clock = SYSPAGE_ENTRY(qtime)->intr;
    r = InterruptQuery(_NTO_INTR_QUERY_SOURCE, clock, 0, &d);
    int ok = r == 0 && d.data.source.num_handlers == 1 && d.deliver_count > 0 && d.deliver_time > 0;
    check("InterruptQuery(SOURCE) of the clock: the kernel's, counted", ok);
    if (!ok)
        printf("drvtest:   clock %d: %d handlers %u count %llu time %llu\n", clock, r, d.data.source.num_handlers,
           (unsigned long long)d.deliver_count, (unsigned long long)d.deliver_time);

    /* Information: CPU 0's clock interrupts over a quiet second (QNX 8 is
     * tickless: few). */
    uint64_t before = d.deliver_count;
    sleep(1);
    if (InterruptQuery(_NTO_INTR_QUERY_SOURCE, clock, 0, &d) == 0) {
        printf("drvtest: clock interrupts on CPU 0 in a quiet second: %llu\n",
               (unsigned long long)(d.deliver_count - before));
    }

    struct sigevent ev;
    SIGEV_INTR_INIT(&ev);
    SIGEV_PULSE_INIT(&ev, srv_coid, -1, _PULSE_CODE_MINAVAIL, 0);
    errno = 0;
    check("InterruptAttachEvent(_NTO_INTR_FLAGS_EXCLUSIVE) on a shared vector: EBUSY",
          InterruptAttachEvent(attached_vector, &ev, _NTO_INTR_FLAGS_EXCLUSIVE) == -1 && errno == EBUSY);
}

static void unblock(void)
{
    pthread_t t;
    pthread_create(&t, NULL, ist, NULL);
    while (ist_state == 0) {
        usleep(1000);
    }
    check("InterruptAttachThread()", ist_state == 1);
    if (ist_state != 1) {
        pthread_join(t, NULL);
        return;
    }
    usleep(20000); /* into InterruptWait() */
    query();
    check("InterruptUnblock()", InterruptUnblock(intr_id) != -1);
    for (int i = 0; i < 1000 && ist_state == 1; i++) {
        usleep(1000);
    }
    check("InterruptWait() returns after InterruptUnblock()", ist_state == 2);
    pthread_join(t, NULL);
    errno = 0;
    check("InterruptUnblock(no such id) fails", InterruptUnblock(intr_id) == -1);
    int b = 0;
    errno = 0;
    check("InterruptCtl(): ENOSYS", InterruptCtl(intr_id, 0, &b, sizeof b) == -1 && errno == ENOSYS);
}

static const struct sigevent *handler(void *area, int id)
{
    (void)area;
    (void)id;
    return NULL;
}

/* As procnto: a handler function is EINVAL; the synthetic vectors up to
 * _NTO_INTR_SPARE (bit 15 aside) ENOSYS, beyond them EINVAL. */
static void attach_errors(void)
{
    struct sigevent ev;
    SIGEV_INTR_INIT(&ev);
    errno = 0;
    check("InterruptAttach() with a handler: EINVAL", InterruptAttach(0, handler, NULL, 0, 0) == -1 && errno == EINVAL);
    errno = 0;
    check("a synthetic vector: ENOSYS", InterruptAttachEvent(0x7fff0000, &ev, 0) == -1 && errno == ENOSYS);
    errno = 0;
    check("synthetic 0x7fff8002: ENOSYS", InterruptAttachEvent(0x7fff8002, &ev, 0) == -1 && errno == ENOSYS);
    errno = 0;
    check("beyond the synthetic vectors: EINVAL", InterruptAttachEvent(0x7fff0003, &ev, 0) == -1 && errno == EINVAL);
    /* The clock's vector is the kernel's: EBUSY (review item SCHED-13; the
     * kernel answered EAGAIN and kept the failed attachment), and again
     * the same, nothing left behind. */
    int clock = SYSPAGE_ENTRY(qtime)->intr;
    errno = 0;
    check("the clock's vector: EBUSY", InterruptAttachEvent(clock, &ev, _NTO_INTR_FLAGS_TRK_MSK) == -1 && errno == EBUSY);
    errno = 0;
    check("the clock's vector again: EBUSY", InterruptAttachEvent(clock, &ev, _NTO_INTR_FLAGS_TRK_MSK) == -1 && errno == EBUSY);
    /* from every CPU: the clock's vector is CPU 0's instance of a per-CPU
     * interrupt, and the kernel answered EINVAL from the others */
    int ncpu = _syspage_ptr->num_cpu, all = 1;
    for (int cpu = 0; cpu < ncpu; cpu++) {
        ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)(1u << cpu));
        sched_yield();
        errno = 0;
        all &= InterruptAttachEvent(clock, &ev, _NTO_INTR_FLAGS_TRK_MSK) == -1 && errno == EBUSY;
    }
    ThreadCtl(_NTO_TCTL_RUNMASK, (void *)(uintptr_t)((1u << ncpu) - 1));
    check("the clock's vector from every CPU: EBUSY", all);
}

int main(void)
{
    attach_errors();
    cache();
    verify();
    unblock();
    printf("drvtest: %s\n", fails ? "FAILED" : "passed");
    return fails != 0;
}

/* Variables and small functions of the parts of the kernel left out. */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/bus.h>
#include <sys/conf.h>
#include <sys/interrupt.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/counter.h>
#include <sys/malloc.h>
#include <sys/mutex.h>
#include <sys/pcpu.h>
#include <sys/proc.h>
#include <sys/resource.h>
#include <sys/resourcevar.h>
#include <sys/smp.h>
#include <sys/sched.h>
#include <sys/selinfo.h>
#include <sys/ucred.h>
#include <sys/vmmeter.h>
#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_page.h>
#include <vm/vm_domainset.h>
#include <vm/vm_pagequeue.h>
#include <vm/vm_phys.h>
#include <machine/atomic.h>

#include "bsdkern.h"

MALLOC_DEFINE(M_IOV, "iov", "large iov's");
MALLOC_DEFINE(M_MOUNT, "mount", "vfs mount structure");
MALLOC_DEFINE(M_SESSION, "session", "session header");
MALLOC_DEFINE(M_SUBPROC, "subproc", "proc subparts");

DPCPU_DEFINE(sbintime_t, hardclocktime);

long physmem = 262144;
volatile long ticksl;
int kstack_pages = 4;
bool lse_supported;
int unmapped_buf_allowed;
int hc_source_mask;
int log_open;
struct vm_domain vm_dom[MAXMEMDOM];
struct vmmeter vm_cnt;
long realmem = 262144;

void timerfd_jumped(void) { }
void vm_wait_domain(int d) { (void)d; bsdkern_yield(); }
int vm_fault_disable_pagefaults(void) { return (0); }
void vm_fault_enable_pagefaults(int s) { (void)s; }
void resettodr(void) { }
void timekeep_push_vdso(void) { }

int
vm_domainset_iter_policy_init(struct vm_domainset_iter *di,
    struct domainset *ds, int *domain, int *flags)
{
	(void)di;
	(void)ds;
	(void)flags;
	*domain = 0;
	return (0);
}

int
vm_domainset_iter_policy_ref_init(struct vm_domainset_iter *di,
    struct domainset_ref *dr, int *domain, int *flags)
{
	(void)di;
	(void)dr;
	(void)flags;
	*domain = 0;
	return (0);
}

int
vm_domainset_iter_policy(struct vm_domainset_iter *di, int *domain)
{
	(void)di;
	(void)domain;
	return (ENOMEM);
}

/* vm_meter.c's counters: nothing reads them here. */
void
bsdkern_vm_counters(void)
{
	counter_u64_t *c = (counter_u64_t *)&vm_cnt;
	size_t i;

	for (i = 0; i < VM_METER_NCOUNTERS; i++)
		c[i] = EARLY_COUNTER;
}

/* Poll: the server's business, once it serves sockets. */
void selrecord(struct thread *td, struct selinfo *sip) { (void)td; (void)sip; }
void seldrain(struct selinfo *sip) { (void)sip; }
/*
 * A wake-up of a selinfo: the server's (netd's poll(), bsdkern_sel_wakeup in
 * its Rust), or nothing for a server that has no poll() over FreeBSD's
 * selinfos (usbd).
 */
void bsdkern_sel_wakeup(void *) __attribute__((weak));
void selwakeuppri(struct selinfo *sip, int pri) { (void)pri; if (bsdkern_sel_wakeup != NULL) bsdkern_sel_wakeup(sip); }
void selwakeup(struct selinfo *sip) { selwakeuppri(sip, 0); }

/* Software interrupts: a thread each. */
struct swi {
	driver_intr_t	*handler;
	void		*arg;
	void		*parker;
	volatile u_int	pending;
};

static void
swi_thread(void *a)
{
	struct swi *s = a;

	for (;;) {
		while (atomic_swap_int(&s->pending, 0) == 0)
			bsdkern_park(s->parker);
		s->handler(s->arg);
	}
}

int
swi_add(struct intr_event **eventp, const char *name, driver_intr_t handler,
    void *arg, int pri, enum intr_type flags, void **cookiep)
{
	struct swi *s = bsdkern_zalloc(sizeof(*s));

	(void)pri;
	(void)flags;
	if (eventp != NULL)
		*eventp = NULL;
	s->handler = handler;
	s->arg = arg;
	s->parker = bsdkern_parker_new();
	if (cookiep != NULL)
		*cookiep = s;
	return (kthread_add(swi_thread, s, NULL, NULL, 0, 0, "swi %s", name));
}

void
swi_sched(void *cookie, int flags)
{
	struct swi *s = cookie;

	(void)flags;
	atomic_store_rel_int(&s->pending, 1);
	bsdkern_unpark(s->parker);
}

/* The CPUs are slots: nothing to start. */
void cpu_mp_setmaxid(void) { mp_maxid = mp_ncpus - 1; }
int cpu_mp_probe(void) { return (mp_ncpus > 1); }
void cpu_mp_start(void) { }
void cpu_mp_announce(void) { }
struct cpu_group *cpu_topo(void) { return (NULL); }
void ipi_cpu(int cpu, u_int ipi) { (void)cpu; (void)ipi; }
void ipi_selected(cpuset_t cpus, u_int ipi) { (void)cpus; (void)ipi; }

/* One user. */
static struct uidinfo uidinfo0;
struct uidinfo *uifind(uid_t uid) { (void)uid; return (&uidinfo0); }
void uihold(struct uidinfo *ui) { (void)ui; }
void uifree(struct uidinfo *ui) { (void)ui; }
int chgsbsize(struct uidinfo *ui, u_int *hiwat, u_int to, rlim_t max) { (void)ui; (void)max; *hiwat = to; return (1); }
int chgkqcnt(struct uidinfo *ui, int diff, rlim_t max) { (void)ui; (void)diff; (void)max; return (1); }
int chgproccnt(struct uidinfo *ui, int diff, rlim_t max) { (void)ui; (void)diff; (void)max; return (1); }
rlim_t lim_cur_proc(struct proc *p, int which) { (void)p; (void)which; return (RLIM_INFINITY); }
rlim_t lim_max(struct thread *td, int which) { (void)td; (void)which; return (RLIM_INFINITY); }

/*
 * devctl(4)'s events (attach, detach: "USB" "DEVICE" "ATTACH" ...): to the
 * server, if it listens (usbd: its clients' hot-plug), else nowhere.
 */
void bsdkern_devctl(const char *, const char *, const char *, const char *) __weak_symbol;
void
devctl_notify(const char *s, const char *t, const char *e, const char *d)
{
	if (bsdkern_devctl != NULL)
		bsdkern_devctl(s, t, e, d);
}
void kdb_backtrace(void) { }

/* Devices (make_dev() and the rest): cdev.c. */

/*
 * The network stack's epoch, which generic code mentions (subr_gtaskqueue.c,
 * for network tasks): netd's stack defines it; elsewhere it stays null.
 */
#include <sys/epoch.h>
epoch_t net_epoch_preempt __weak_symbol;

/* The boot flags (RB_*) and the loader's command line: none in a server. */
#include <sys/boot.h>
int boothowto;

int
boot_parse_cmdline(char *cmdline)
{
	(void)cmdline;
	return (0);
}

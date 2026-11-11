/* The small things FreeBSD's kernel code expects of the rest of its kernel. */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/cons.h>
#include <sys/domainset.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/msgbuf.h>
#include <sys/mutex.h>
#include <sys/pcpu.h>
#include <sys/proc.h>
#include <sys/random.h>
#include <sys/sched.h>
#include <sys/smp.h>
#include <sys/stdarg.h>
#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/vm_page.h>
#include <vm/vm_phys.h>

#include "bsdkern.h"

const char version[] = "OsCraft";
const char ostype[] = "OsCraft";
const char osrelease[] = "15.0";
const int osreldate = 1500000;
char kern_ident[] = "OSCRAFT";
const char compiler_version[] = "clang";
int bootverbose;
int cold = 1;
int scheduler_stopped_flag;
char static_env[1];
char static_hints[1];
int kdb_active;
const char *panicstr;
int cn_mute;
struct mtx msgbuf_lock_unused;

/* The kernel's cross-address copies: netd moves data itself, by message. */
int
copyin(const void *from, void *to, size_t n)
{
	memcpy(to, from, n);
	return (0);
}

int
copyout(const void *from, void *to, size_t n)
{
	memcpy(to, from, n);
	return (0);
}

int
copyinstr(const void *from, void *to, size_t max, size_t *done)
{
	size_t n = strnlen(from, max);

	if (n == max)
		return (ENAMETOOLONG);
	memcpy(to, from, n + 1);
	if (done != NULL)
		*done = n + 1;
	return (0);
}

void
cnputc(int c)
{
	char b = c;

	bsdkern_console(&b, 1);
}

void
cnputs(const char *s)
{
	bsdkern_console(s, strlen(s));
}

/* libc's, in this process: the object and symbol an address is in. */
struct bsdkern_dl_info {
	const char	*dli_fname;
	void		*dli_fbase;
	const char	*dli_sname;
	void		*dli_saddr;
};
int	dladdr(const void *, struct bsdkern_dl_info *);

/*
 * Who called panic(), after its message: FreeBSD's kernel code keeps frame
 * pointers, so the callers are a chain of [frame pointer, return address]
 * pairs up the stack, named by the dynamic linker (the server exports its
 * symbols, its modules theirs).
 */
static size_t
backtrace(char *b, size_t room)
{
	struct bsdkern_dl_info info;
	uintptr_t *fp, *next, pc;
	size_t n = 0;
	int i;

	fp = __builtin_frame_address(0);
	for (i = 0; i < 24 && fp != NULL && n < room; i++) {
		next = (uintptr_t *)fp[0];
		pc = fp[1];
		if (pc == 0)
			break;
		if (dladdr((void *)pc, &info) != 0 && info.dli_sname != NULL)
			n += snprintf(b + n, room - n, "\n  %s+%#lx",
			    info.dli_sname,
			    (u_long)(pc - (uintptr_t)info.dli_saddr));
		else
			n += snprintf(b + n, room - n, "\n  %#lx", (u_long)pc);
		/* The stack grows down: a caller's frame is above. */
		if (next <= fp || ((uintptr_t)next & 0xf) != 0)
			break;
		fp = next;
	}
	return (n);
}

void
vpanic(const char *fmt, va_list ap)
{
	static char b[2048];
	size_t n;

	n = vsnprintf(b, 256, fmt, ap);
	panicstr = b;
	backtrace(b + n, sizeof(b) - n);
	bsdkern_panic(b);
}

void
panic(const char *fmt, ...)
{
	va_list ap;

	va_start(ap, fmt);
	vpanic(fmt, ap);
}

/*
 * Interrupts off, in a kernel. Spin mutexes here take their critical section
 * themselves, also leaving it when mi_switch()'s caller's lock is released,
 * so mi_switch()'s spinlock_exit() has nothing to pair with.
 */
void
spinlock_enter(void)
{
}

void
spinlock_exit(void)
{
}

/* td_lock's own, moved by the sleep queues. */
void
thread_lock_set(struct thread *td, struct mtx *new)
{
	struct mtx *lock = td->td_lock;

	td->td_lock = new;
	mtx_unlock_spin(lock);
}

void
thread_lock_block_wait(struct thread *td)
{
	(void)td;
}

void
_thread_lock(struct thread *td)
{
	struct mtx *m;

	for (;;) {
		m = td->td_lock;
		mtx_lock_spin(m);
		if (m == td->td_lock)
			return;
		mtx_unlock_spin(m);
	}
}

/* Random numbers: libc's, in the absence of FreeBSD's device. */
void
arc4rand(void *p, u_int n, int reseed)
{
	(void)reseed;
	arc4random_buf(p, n);
}

bool
is_random_seeded(void)
{
	return (true);
}

void
read_random(void *p, u_int n)
{
	arc4random_buf(p, n);
}

/* One domain. */
struct domainset domainset_prefer[MAXMEMDOM];
struct domainset domainset_roundrobin;
struct domainset domainset_fixed[MAXMEMDOM];
int vm_ndomains = 1;
domainset_t vm_min_domains;

/* pcpu slots; each slot's memory comes from bsdkern. */
void
bsdkern_pcpus_init(int cpus)
{
	struct pcpu *pc;
	void *dpcpu;
	int i;

	bsdkern_slots_init(cpus);
	for (i = 0; i < cpus; i++) {
		pc = bsdkern_slot_pcpu(i);
		pcpu_init(pc, i, sizeof(struct pcpu));
		/*
		 * With room for modules' per-CPU variables after the program's
		 * (link_dl.c): FreeBSD's dpcpu_alloc() assumes its linker
		 * script padded the set with DPCPU_MODMIN, which this one isn't.
		 */
		dpcpu = bsdkern_zalloc(roundup2(DPCPU_BYTES, 64) + BSDKERN_DPCPU_MODSPACE);
		dpcpu_init(dpcpu, i);
		CPU_SET(i, &all_cpus);
	}
	bsdkern_slot_attach();
}

void
cpu_pcpu_init(struct pcpu *pc, int cpuid, size_t size)
{
	(void)pc;
	(void)cpuid;
	(void)size;
}

/*
 * Small services of a kernel that any server's FreeBSD code may call (the
 * USB core's): its threads aren't suspended by anyone (kthread_suspend()
 * is for shutdown), there's no root to wait for, no reboot in progress, and
 * "user" memory is this process's (copyout()'s, word by word).
 */
int rebooting;

int
kthread_suspend(struct thread *td, int timo)
{
	(void)td;
	(void)timo;
	return (0);
}

void
kthread_suspend_check(void)
{
}

struct root_hold_token *
root_mount_hold(const char *identifier)
{
	(void)identifier;
	return (NULL);
}

void
root_mount_rel(struct root_hold_token *h)
{
	(void)h;
}

int
suword16(volatile void *base, int word)
{
	*(volatile uint16_t *)base = word;
	return (0);
}

int
suword32(volatile void *base, int32_t word)
{
	*(volatile int32_t *)base = word;
	return (0);
}

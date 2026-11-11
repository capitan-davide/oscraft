/*
 * A scheduler for FreeBSD's shim (sys/sched.h), in place of ULE, for a user
 * process on OsCraft. kern_synch.c and subr_sleepqueue.c stay as they are:
 * they mark a thread inhibited and call sched_switch(), or sched_wakeup()
 * to end it. There are no run queues: the kernel schedules, and a thread
 * that switches away parks until it is no longer inhibited. io-sock instead
 * edits sleepq_switch() to wait on a condition.
 *
 * Compiled with the kernel's flags and headers (make bsdnet), since it
 * reaches into struct thread.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/mutex.h>
#include <sys/proc.h>
#include <sys/sched.h>
#include <machine/atomic.h>

#include "bsdkern.h"

static void
oscraft_switch(struct thread *td, int flags)
{
	struct bsdkern_td *b = td->td_emuldata;
	bool sleeping = TD_IS_INHIBITED(td);

	(void)flags;
	/* Before the lock goes: a wake-up can come from then on. */
	if (sleeping)
		atomic_store_rel_int(&b->inhibited, 1);
	mtx_unlock_spin(td->td_lock);
	if (!sleeping) {
		bsdkern_yield();
		return;
	}
	while (atomic_load_acq_int(&b->inhibited) != 0)
		bsdkern_park(b->parker);
}

/*
 * The thread has stopped being inhibited; its lock, held here, is the sleep
 * chain's. It gets its own back, then runs. The chain's is released unless
 * the caller keeps it.
 */
static void
oscraft_wakeup(struct thread *td, int flags)
{
	struct bsdkern_td *b = td->td_emuldata;
	struct mtx *held = td->td_lock;
	void *start = b->start;

	td->td_lock = &b->lock;
	TD_SET_RUNNING(td);
	if (start != NULL) {
		/* A softclock thread: made stopped, and first woken by the clock. */
		b->start = NULL;
		if (bsdkern_thread_spawn(start, td->td_name) != 0)
			panic("sched_wakeup: no thread");
	} else {
		atomic_store_rel_int(&b->inhibited, 0);
		bsdkern_unpark(b->parker);
	}
	if ((flags & (SRQ_HOLD | SRQ_HOLDTD)) == 0)
		mtx_unlock_spin(held);
}

int sched_oscraft_load(void) { return (0); }
int sched_oscraft_rr_interval(void) { return (hz / 10); }
bool sched_oscraft_runnable(void) { return (false); }
void sched_oscraft_exit(struct proc *p, struct thread *c) { (void)p; (void)c; }
void sched_oscraft_fork(struct thread *t, struct thread *c) { (void)t; (void)c; }
void sched_oscraft_fork_exit(struct thread *t) { (void)t; }
void sched_oscraft_class(struct thread *t, int c) { t->td_pri_class = c; }
void sched_oscraft_nice(struct proc *p, int n) { p->p_nice = n; }
void sched_oscraft_ap_entry(void) { }
void sched_oscraft_exit_thread(struct thread *t, struct thread *c) { (void)t; (void)c; }
u_int sched_oscraft_estcpu(struct thread *t) { (void)t; return (0); }
void sched_oscraft_fork_thread(struct thread *t, struct thread *c) { (void)t; (void)c; }
void sched_oscraft_ithread_prio(struct thread *t, u_char p) { t->td_base_ithread_pri = p; t->td_priority = p; }
void sched_oscraft_lend_prio(struct thread *t, u_char p) { t->td_priority = p; }
void sched_oscraft_lend_user_prio(struct thread *t, u_char p) { t->td_lend_user_pri = p; }
void sched_oscraft_lend_user_prio_cond(struct thread *t, u_char p) { t->td_lend_user_pri = p; }
fixpt_t sched_oscraft_pctcpu(struct thread *t) { (void)t; return (0); }
void sched_oscraft_prio(struct thread *t, u_char p) { t->td_priority = p; }
void sched_oscraft_sleep(struct thread *t, int p) { (void)t; (void)p; }
void sched_oscraft_switch(struct thread *t, int f) { oscraft_switch(t, f); }
void sched_oscraft_throw(struct thread *t) { (void)t; panic("sched_throw"); }
void sched_oscraft_unlend_prio(struct thread *t, u_char p) { t->td_priority = p; }
void sched_oscraft_user_prio(struct thread *t, u_char p) { t->td_user_pri = p; }
void sched_oscraft_userret_slowpath(struct thread *t) { t->td_priority = t->td_user_pri; }

/*
 * sched_add() starts a thread made stopped, or wakes one that waits for
 * interrupts (TD_SET_IWAIT, then mi_switch()); its lock, whichever it is,
 * is released unless the caller keeps it.
 */
void
sched_oscraft_add(struct thread *td, int flags)
{
	struct bsdkern_td *b = td->td_emuldata;
	struct mtx *held = td->td_lock;
	void *start = b->start;

	td->td_lock = &b->lock;
	TD_SET_RUNNING(td);
	if (start != NULL) {
		b->start = NULL;
		if (bsdkern_thread_spawn(start, td->td_name) != 0)
			panic("sched_add: no thread");
	} else {
		atomic_store_rel_int(&b->inhibited, 0);
		bsdkern_unpark(b->parker);
	}
	if ((flags & SRQ_HOLDTD) == 0)
		mtx_unlock_spin(held);
}

struct thread *sched_oscraft_choose(void) { return (NULL); }
void sched_oscraft_clock(struct thread *t, int c) { (void)t; (void)c; }
void sched_oscraft_idletd(void *d) { (void)d; }
void sched_oscraft_preempt(struct thread *t) { (void)t; }
void sched_oscraft_relinquish(struct thread *t) { (void)t; bsdkern_yield(); }
void sched_oscraft_rem(struct thread *t) { (void)t; }
void sched_oscraft_wakeup(struct thread *t, int f) { oscraft_wakeup(t, f); }
void sched_oscraft_bind(struct thread *t, int c) { ((struct bsdkern_td *)t->td_emuldata)->bound = 1; bsdkern_bind(c); }
void sched_oscraft_unbind(struct thread *t) { ((struct bsdkern_td *)t->td_emuldata)->bound = 0; bsdkern_bind(-1); }
int sched_oscraft_is_bound(struct thread *t) { return (((struct bsdkern_td *)t->td_emuldata)->bound); }
void sched_oscraft_affinity(struct thread *t) { (void)t; }
int sched_oscraft_sizeof_proc(void) { return (sizeof(struct proc)); }
int sched_oscraft_sizeof_thread(void) { return (sizeof(struct thread)); }
char *sched_oscraft_tdname(struct thread *t) { return (t->td_name); }
void sched_oscraft_clear_tdname(struct thread *t) { (void)t; }
int sched_oscraft_find_l2_neighbor(int c) { (void)c; return (-1); }
void sched_oscraft_init_ap(void) { }
void sched_oscraft_init(void) { }
void sched_oscraft_setup(void) { }
void sched_oscraft_initticks(void) { }
void sched_oscraft_sysinit(void) { }

/*
 * sched_shim.c would dispatch these through ifuncs, resolved when libc
 * starts, before anything chose a scheduler. There is only one here.
 */
#define	BIND(args, ret_type, fn, ...)					\
	extern __typeof(sched_##fn) sched_##fn				\
	    __attribute__((alias("sched_oscraft_" #fn)));
SCHED_ITF_DISPATCH(BIND, )

struct cpu_group *cpu_top;

void
sched_init(void)
{
}

/* For the slots: sched_pin() keeps a thread on its slot. */
static int
pinned(void)
{
	return (curthread != NULL && curthread->td_pinned != 0);
}

void
bsdkern_sched_init(void)
{
	bsdkern_slot_pin_query(pinned);
}

/*
 * FreeBSD's threads as threads of this process. As in io-sock, every kernel
 * thread is a pthread with a real struct thread, and every "process" is
 * proc0: kproc_create() makes a thread. curthread is thread-local here, so
 * no table of threads is needed (io-sock keeps tds_all as well).
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/mutex.h>
#include <sys/proc.h>
#include <sys/refcount.h>
#include <sys/unistd.h>

const char	*getprogname(void);	/* libc's: the server's name */
#include <sys/jail.h>
#include <sys/loginclass.h>
#include <sys/resource.h>
#include <sys/resourcevar.h>
#include <sys/sleepqueue.h>
#include <sys/sysent.h>
#include <sys/ucred.h>
#include <sys/stdarg.h>

#include "bsdkern.h"

struct proc proc0;
struct thread0_storage thread0_st;
static struct bsdkern_td thread0_b;
static volatile u_int next_tid = 100001;
static bool cold_threads = true;

static struct plimit limit0;
static struct loginclass class0;
static struct sysentvec sysent0;

static void
thread_setup(struct thread *td, struct bsdkern_td *b)
{
	mtx_init(&b->lock, "thread lock", NULL, MTX_SPIN | MTX_NEW);
	b->parker = bsdkern_parker_new();
	td->td_emuldata = b;
	td->td_lock = &b->lock;
	td->td_proc = &proc0;
	td->td_limit = &limit0;
	td->td_tid = atomic_fetchadd_int(&next_tid, 1);
	td->td_ucred = proc0.p_ucred;
	if (!cold_threads)
		td->td_sleepqueue = sleepq_alloc();
	/* For timed sleeps (sleepq_set_timeout()), as thread_init() does. */
	callout_init(&td->td_slpcallout, 1);
	TD_SET_RUNNING(td);
}

/*
 * What proc0_init() does that matters here, once UMA is up: thread0's sleep
 * queue, and the credentials every thread shares.
 */
static void
proc0_start(void *dummy)
{
	struct ucred *cr;

	(void)dummy;
	cold_threads = false;
	thread0.td_sleepqueue = sleepq_alloc();
	mtx_init(&proc0.p_mtx, "process lock", NULL, MTX_DEF | MTX_DUPOK);
	cr = crget();
	cr->cr_ngroups = 1;
	cr->cr_prison = &prison0;
	prison_hold(&prison0);
	refcount_init(&class0.lc_refcount, 1);
	strlcpy(class0.lc_name, "default", sizeof(class0.lc_name));
	cr->cr_loginclass = &class0;
	cr->cr_uidinfo = uifind(0);
	cr->cr_ruidinfo = uifind(0);
	proc0.p_ucred = cr;
	thread0.td_ucred = crhold(cr);
}
SYSINIT(oscraft_proc0, SI_SUB_KMEM, SI_ORDER_ANY + 1, proc0_start, NULL);

/* The first thread; bsdkern_set_curthread() is the Rust side's. */
void
bsdkern_thread0(void)
{
	for (int i = 0; i < RLIM_NLIMITS; i++)
		limit0.pl_rlimit[i].rlim_cur = limit0.pl_rlimit[i].rlim_max =
		    RLIM_INFINITY;
	proc0.p_limit = &limit0;
	proc0.p_sysent = &sysent0;
	proc0.p_pid = 0;
	proc0.p_state = PRS_NORMAL;
	proc0.p_numthreads = 1;
	strlcpy(proc0.p_comm, getprogname(), sizeof(proc0.p_comm));
	thread_setup(&thread0, &thread0_b);
	strlcpy(thread0.td_name, "main", sizeof(thread0.td_name));
	bsdkern_set_curthread(&thread0);
}

struct start {
	void		(*func)(void *);
	void		*arg;
	struct thread	*td;
};

/* The new pthread's first call, from Rust. */
void
bsdkern_thread_start(void *a)
{
	struct start s = *(struct start *)a;

	bsdkern_free(a);
	bsdkern_set_curthread(s.td);
	bsdkern_slot_attach();
	s.func(s.arg);
}

int
kthread_add(void (*func)(void *), void *arg, struct proc *p,
    struct thread **newtdp, int flags, int pages, const char *fmt, ...)
{
	struct thread *td;
	struct bsdkern_td *b;
	struct start *s;
	va_list ap;

	(void)p;
	(void)pages;
	td = bsdkern_zalloc(sizeof(*td));
	b = bsdkern_zalloc(sizeof(*b));
	s = bsdkern_zalloc(sizeof(*s));
	thread_setup(td, b);
	va_start(ap, fmt);
	vsnprintf(td->td_name, sizeof(td->td_name), fmt, ap);
	va_end(ap);
	s->func = func;
	s->arg = arg;
	s->td = td;
	if (newtdp != NULL)
		*newtdp = td;
	/* A stopped thread is for sched_add() to start. */
	if ((flags & RFSTOPPED) != 0) {
		b->start = s;
		TD_SET_CAN_RUN(td);
		return (0);
	}
	return (bsdkern_thread_spawn(s, td->td_name));
}

int
kproc_create(void (*func)(void *), void *arg, struct proc **newpp, int flags,
    int pages, const char *fmt, ...)
{
	char name[MAXCOMLEN + 1];
	va_list ap;

	va_start(ap, fmt);
	vsnprintf(name, sizeof(name), fmt, ap);
	va_end(ap);
	if (newpp != NULL)
		*newpp = &proc0;
	return (kthread_add(func, arg, NULL, NULL, flags, pages, "%s", name));
}

int
kproc_kthread_add(void (*func)(void *), void *arg, struct proc **procptr,
    struct thread **tdptr, int flags, int pages, const char *procname,
    const char *fmt, ...)
{
	char name[MAXCOMLEN + 1];
	va_list ap;

	(void)procname;
	va_start(ap, fmt);
	vsnprintf(name, sizeof(name), fmt, ap);
	va_end(ap);
	if (procptr != NULL)
		*procptr = &proc0;
	return (kthread_add(func, arg, NULL, tdptr, flags, pages, "%s", name));
}

void
kthread_exit(void)
{
	bsdkern_thread_exit();
}

void
kproc_exit(int ecode)
{
	(void)ecode;
	bsdkern_thread_exit();
}

/* A thread FreeBSD's code didn't start, at its first call into it. */
void
bsdkern_attach(void)
{
	struct thread *td;
	struct bsdkern_td *b;

	if (curthread != NULL)
		return;
	td = bsdkern_zalloc(sizeof(*td));
	b = bsdkern_zalloc(sizeof(*b));
	thread_setup(td, b);
	strlcpy(td->td_name, "server", sizeof(td->td_name));
	bsdkern_set_curthread(td);
	bsdkern_slot_attach();
	td->td_ucred = crhold(proc0.p_ucred);
}

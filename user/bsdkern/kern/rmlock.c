/*
 * rm locks as sx locks, as io-sock's rm_init_flags() makes them
 * ("rmlock_sx"), in place of kern_rmlock.c: its read path needs per-CPU
 * tracking that a preemptible critical section can't give.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/mutex.h>
#include <sys/rmlock.h>
#include <sys/sx.h>

#define	RM_SX(rm)	(&(rm)->_rm_lock._rm_lock_sx)

void
rm_init_flags(struct rmlock *rm, const char *name, int opts)
{
	(void)opts;
	bzero(rm, sizeof(*rm));
	rm->lock_object.lo_name = name;
	rm->lock_object.lo_flags = LO_INITIALIZED;
	sx_init_flags(RM_SX(rm), name, 0);
}

void
rm_init(struct rmlock *rm, const char *name)
{
	rm_init_flags(rm, name, 0);
}

void
rm_destroy(struct rmlock *rm)
{
	sx_destroy(RM_SX(rm));
}

int
rm_wowned(const struct rmlock *rm)
{
	return (sx_xlocked(__DECONST(struct sx *, RM_SX(rm))));
}

void
rm_sysinit(const void *arg)
{
	const struct rm_args *args = arg;

	rm_init_flags(args->ra_rm, args->ra_desc, args->ra_flags);
}

void
_rm_wlock(struct rmlock *rm)
{
	sx_xlock(RM_SX(rm));
}

void
_rm_wunlock(struct rmlock *rm)
{
	sx_xunlock(RM_SX(rm));
}

int
_rm_rlock(struct rmlock *rm, struct rm_priotracker *tracker, int trylock)
{
	(void)tracker;
	if (trylock)
		return (sx_try_slock(RM_SX(rm)));
	sx_slock(RM_SX(rm));
	return (1);
}

void
_rm_runlock(struct rmlock *rm, struct rm_priotracker *tracker)
{
	(void)tracker;
	sx_sunlock(RM_SX(rm));
}

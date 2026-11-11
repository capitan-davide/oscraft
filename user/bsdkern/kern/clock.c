/*
 * The clock: a thread for hardclock's work on callouts and timecounters.
 * Time comes from cntvct_el0, as in io-sock's qnx_get_timecount().
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/callout.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/mutex.h>
#include <sys/pcpu.h>
#include <sys/proc.h>
#include <sys/sched.h>
#include <sys/smp.h>
#include <sys/time.h>
#include <sys/timetc.h>
#include <machine/atomic.h>
#include <machine/cpufunc.h>

#include "bsdkern.h"

volatile int ticks;
DPCPU_DECLARE(sbintime_t, hardclocktime);

static u_int
get_timecount(struct timecounter *tc)
{
	uint64_t v;

	(void)tc;
	__asm __volatile("isb; mrs %0, cntvct_el0" : "=r"(v));
	return ((u_int)v);
}

static struct timecounter counter = {
	.tc_get_timecount = get_timecount,
	.tc_counter_mask = ~0u,
	.tc_name = "cntvct",
	.tc_quality = 1000,
};

static void
timecounter_start(void *dummy)
{
	uint64_t f;

	(void)dummy;
	__asm __volatile("mrs %0, cntfrq_el0" : "=r"(f));
	counter.tc_frequency = f;
	tc_init(&counter);
}
SYSINIT(oscraft_tc, SI_SUB_LOCK, SI_ORDER_ANY, timecounter_start, NULL);

static void
clock_thread(void *dummy)
{
	int cpu;

	(void)dummy;
	for (;;) {
		bsdkern_nanosleep(1000000000 / hz);
		atomic_add_int(&ticks, 1);
		tc_ticktock(1);
		CPU_FOREACH(cpu) {
			sched_bind(curthread, cpu);
			critical_enter();
			/*
			 * As handleevents() in kern_clocksource.c: callout_when()
			 * counts from it, and with 0 every time would be past,
			 * and a callout that sets itself again would never end.
			 */
			DPCPU_SET(hardclocktime, sbinuptime());
			callout_process(sbinuptime());
			critical_exit();
			sched_unbind(curthread);
		}
	}
}

void
bsdkern_clock_start(void)
{
	if (kthread_add(clock_thread, NULL, NULL, NULL, 0, 0, "clock") != 0)
		panic("no clock");
}

void
cpu_new_callout(int cpu, sbintime_t bt, sbintime_t bt_opt)
{
	(void)cpu;
	(void)bt;
	(void)bt_opt;
}

int
tvtohz(struct timeval *tv)
{
	int64_t us = (int64_t)tv->tv_sec * 1000000 + tv->tv_usec;

	return ((int)MIN(us * hz / 1000000 + 1, INT_MAX));
}

void
DELAY(int us)
{
	bsdkern_nanosleep((uint64_t)us * 1000);
}

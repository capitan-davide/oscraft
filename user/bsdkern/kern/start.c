/*
 * Starting FreeBSD's code in this process: what mi_startup() and the
 * machine-dependent start-up do in a kernel. As in io-sock's main(): set up,
 * then run the SYSINITs of what was linked in.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/linker_set.h>
#include <sys/lock.h>
#include <sys/module.h>
#include <sys/mutex.h>
#include <sys/pcpu.h>
#include <sys/proc.h>
#include <sys/smp.h>
#include <sys/ucred.h>

#include "bsdkern.h"

SET_DECLARE(sysinit_set, struct sysinit);

struct ucred thread0_cred;

static int
compare(const void *a, const void *b)
{
	const struct sysinit *x = *(struct sysinit *const *)a;
	const struct sysinit *y = *(struct sysinit *const *)b;

	if (x->subsystem != y->subsystem)
		return (x->subsystem < y->subsystem ? -1 : 1);
	if (x->order != y->order)
		return (x->order < y->order ? -1 : 1);
	return (0);
}

/* What is run in order, skipping nothing: only what is linked is here. */
static void
sysinits(void)
{
	struct sysinit **list, **p;
	size_t n = SET_COUNT(sysinit_set), i = 0;

	list = bsdkern_zalloc(n * sizeof(*list));
	SET_FOREACH(p, sysinit_set)
		list[i++] = *p;
	bsd_qsort(list, n, sizeof(*list), compare);
	for (i = 0; i < n; i++)
		if (list[i]->subsystem != SI_SUB_DUMMY) {
			if (bootverbose)
				printf("sysinit %x %p\n", list[i]->subsystem,
				    list[i]->func);
			list[i]->func(list[i]->udata);
		}
	bsdkern_free(list);
}

/*
 * The kernel environment the server is started with (its -e options):
 * "name=value" strings, as the loader's (loader.conf: device hints such
 * as hint.pcib.0.disabled, tunables), installed before anything reads it.
 */
static char kenv_buf[4096];
static size_t kenv_pos;

int	bsdkern_kenv_add(const char *);

int
bsdkern_kenv_add(const char *s)
{
	size_t n = strlen(s) + 1;

	if (strchr(s, '=') == NULL || kenv_pos + n + 1 > sizeof(kenv_buf))
		return (EINVAL);
	memcpy(kenv_buf + kenv_pos, s, n);
	kenv_pos += n;
	return (0);
}

/* A variable of the kernel environment, as FreeBSD's code reads it. */
int	bsdkern_kenv_get(const char *, char *, int);

int
bsdkern_kenv_get(const char *name, char *buf, int len)
{
	return (getenv_string(name, buf, len));
}

void
bsdkern_kernel_start(int cpus)
{
	if (kenv_pos != 0)
		init_static_kenv(kenv_buf, 0);
	mp_ncpus = cpus;
	mp_maxid = cpus - 1;
	init_param1();
	init_param2(physmem);
	bsdkern_mem_start();
	bsdkern_vm_counters();
	bsdkern_thread0();
	bsdkern_sched_init();
	bsdkern_pcpus_init(cpus);
	sysinits();
	cold = 0;
	bsdkern_clock_start();
}

/* The modules in the program: kern_linker.c registers them (link_dl.c). */

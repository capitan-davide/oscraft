/*
 * FreeBSD's kernel linker (kern_linker.c) over the dynamic loader: a module
 * is a shared object, loaded with dlopen() into the program that holds the
 * stack, its linker sets found by their __start_set_ and __stop_set_
 * symbols, and unloaded with dlclose(). As io-sock's link_elf class (its
 * link_elf_load_file, link_elf_lookup_set, link_elf_unload_file, read with
 * REA), but every module may be unloaded: kern_linker.c's own sequence
 * (MOD_QUIESCE, MOD_UNLOAD, SYSUNINIT) runs first.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/kobj.h>
#include <sys/linker.h>
#include <sys/malloc.h>
#include <sys/module.h>
#include <sys/pcpu.h>
#include <sys/smp.h>

const char	*getprogname(void);	/* libc's: the kernel file's name */

#include "bsdkern.h"
#include "linker_if.h"

/* <dlfcn.h>'s, which the kernel's headers don't include. */
#define	RTLD_NOW	2
#define	RTLD_GLOBAL	0x100
void	*dlopen(const char *, int);
void	*dlsym(void *, const char *);
int	dlclose(void *);
const char *dlerror(void);
typedef struct {
	const char	*dli_fname;
	void		*dli_fbase;
	const char	*dli_sname;
	void		*dli_saddr;
} Dl_info;
int	dladdr(const void *, Dl_info *);

/* What a module says it was built against: netd's module ABI. */
#define	NETD_MODULE_ABI	1

struct dl_file {
	struct linker_file lf;
	void		*handle;	/* NULL: the program itself */
	uintptr_t	dpcpu;		/* its per-CPU room's offset, or 0 */
	size_t		dpcpu_size;
};

/*
 * Modules' per-CPU room, after the program's set in each CPU's block
 * (bsdkern_pcpus_init()): in 64-byte units, a bit each, taken first fit.
 */
#define	DPCPU_UNIT	64
#define	DPCPU_UNITS	(BSDKERN_DPCPU_MODSPACE / DPCPU_UNIT)
static uint64_t dpcpu_used[DPCPU_UNITS / 64];

static uintptr_t
dpcpu_room(void)
{
	return (roundup2(DPCPU_STOP, DPCPU_UNIT));
}

/* The room's offset for `size` bytes, 0 if there is none. */
static uintptr_t
dpcpu_take(size_t size)
{
	size_t n = howmany(size, DPCPU_UNIT), run = 0, i, j;

	for (i = 0; i < DPCPU_UNITS - 1; i++) {
		run = (dpcpu_used[i / 64] >> (i % 64) & 1) ? 0 : run + 1;
		if (run == n) {
			for (j = i + 1 - n; j <= i; j++)
				dpcpu_used[j / 64] |= 1ul << (j % 64);
			return ((i + 1 - n) * DPCPU_UNIT + DPCPU_UNIT);
		}
	}
	return (0);
}

static void
dpcpu_give(uintptr_t off, size_t size)
{
	size_t j, first = off / DPCPU_UNIT - 1;

	for (j = first; j < first + howmany(size, DPCPU_UNIT); j++)
		dpcpu_used[j / 64] &= ~(1ul << (j % 64));
}

static int link_dl_load_file(linker_class_t, const char *, linker_file_t *);

static int
link_dl_lookup_symbol(linker_file_t lf, const char *name, c_linker_sym_t *sym)
{
	struct dl_file *f = (struct dl_file *)lf;
	void *p = dlsym(f->handle != NULL ? f->handle : dlopen(NULL, RTLD_NOW), name);

	if (p == NULL)
		return (ENOENT);
	*sym = (c_linker_sym_t)p;
	return (0);
}

static int
link_dl_symbol_values(linker_file_t lf, c_linker_sym_t sym, linker_symval_t *v)
{
	Dl_info info;

	(void)lf;
	v->value = (caddr_t)sym;
	v->size = 0;
	v->name = dladdr(sym, &info) && info.dli_sname != NULL ? info.dli_sname : "";
	return (0);
}

static int
link_dl_search_symbol(linker_file_t lf, caddr_t value, c_linker_sym_t *sym, long *diff)
{
	Dl_info info;

	(void)lf;
	if (!dladdr(value, &info) || info.dli_saddr == NULL)
		return (ENOENT);
	*sym = (c_linker_sym_t)info.dli_saddr;
	*diff = value - (caddr_t)info.dli_saddr;
	return (0);
}

static int
link_dl_each_function_name(linker_file_t lf, linker_function_name_callback_t cb, void *opaque)
{
	(void)lf;
	(void)cb;
	(void)opaque;
	return (0);
}

static int
link_dl_each_function_nameval(linker_file_t lf, linker_function_nameval_callback_t cb, void *opaque)
{
	(void)lf;
	(void)cb;
	(void)opaque;
	return (0);
}

/*
 * A set's bounds, from its __start_set_ and __stop_set_ symbols. The
 * program's own sets aren't looked up so: kern_linker.c takes the kernel's
 * modules from SET_BEGIN(modmetadata_set) when this fails for it.
 */
static int
link_dl_lookup_set(linker_file_t lf, const char *name, void ***start, void ***stop, int *count)
{
	struct dl_file *f = (struct dl_file *)lf;
	char sym[128];
	void **b, **e;

	if (f->handle == NULL)
		return (ENOENT);
	snprintf(sym, sizeof(sym), "__start_set_%s", name);
	b = dlsym(f->handle, sym);
	snprintf(sym, sizeof(sym), "__stop_set_%s", name);
	e = dlsym(f->handle, sym);
	if (b == NULL || e == NULL)
		return (ENOENT);
	if (start != NULL)
		*start = b;
	if (stop != NULL)
		*stop = e;
	if (count != NULL)
		*count = e - b;
	return (0);
}

static void
link_dl_unload(linker_file_t lf)
{
	struct dl_file *f = (struct dl_file *)lf;

	if (f->dpcpu != 0)
		dpcpu_give(f->dpcpu, f->dpcpu_size);
	f->dpcpu = 0;
	if (f->handle != NULL && dlclose(f->handle) != 0)
		printf("link_dl: %s: %s\n", lf->filename, dlerror());
	f->handle = NULL;
}

static int
link_dl_link_preload(linker_class_t cls, const char *name, linker_file_t *result)
{
	(void)cls;
	(void)name;
	(void)result;
	return (ENOENT);
}

static int
link_dl_link_preload_finish(linker_file_t lf)
{
	(void)lf;
	return (0);
}

static long
link_dl_symtab_get(linker_file_t lf, const Elf_Sym **symtab)
{
	(void)lf;
	*symtab = NULL;
	return (0);
}

static long
link_dl_strtab_get(linker_file_t lf, caddr_t *strtab)
{
	(void)lf;
	*strtab = NULL;
	return (0);
}

static kobj_method_t link_dl_methods[] = {
	KOBJMETHOD(linker_lookup_symbol,	link_dl_lookup_symbol),
	KOBJMETHOD(linker_lookup_debug_symbol,	link_dl_lookup_symbol),
	KOBJMETHOD(linker_symbol_values,	link_dl_symbol_values),
	KOBJMETHOD(linker_debug_symbol_values,	link_dl_symbol_values),
	KOBJMETHOD(linker_search_symbol,	link_dl_search_symbol),
	KOBJMETHOD(linker_unload,		link_dl_unload),
	KOBJMETHOD(linker_load_file,		link_dl_load_file),
	KOBJMETHOD(linker_link_preload,		link_dl_link_preload),
	KOBJMETHOD(linker_link_preload_finish,	link_dl_link_preload_finish),
	KOBJMETHOD(linker_lookup_set,		link_dl_lookup_set),
	KOBJMETHOD(linker_each_function_name,	link_dl_each_function_name),
	KOBJMETHOD(linker_each_function_nameval, link_dl_each_function_nameval),
	KOBJMETHOD(linker_symtab_get,		link_dl_symtab_get),
	KOBJMETHOD(linker_strtab_get,		link_dl_strtab_get),
	KOBJMETHOD_END
};

static struct linker_class link_dl_class = {
	"dl", link_dl_methods, sizeof(struct dl_file)
};

/* Whether `name` is defined in the object of `h`, where `own` is. */
static bool
defined_in(void *h, const void *own, const char *name)
{
	Dl_info mine, its;
	void *p = dlsym(h, name);

	return (p != NULL && dladdr(own, &mine) && dladdr(p, &its) &&
	    mine.dli_fbase == its.dli_fbase);
}

/*
 * dlopen() with the program's symbols for the module's references, after
 * checking the module was built for this server's bsdkern (bsdkern_module_abi).
 */
static int
link_dl_load_file(linker_class_t cls, const char *filename, linker_file_t *result)
{
	struct dl_file *f;
	const int *abi;
	void *h;

	(void)cls;
	h = dlopen(filename, RTLD_NOW | RTLD_GLOBAL);
	if (h == NULL) {
		printf("link_dl: %s: %s\n", filename, dlerror());
		return (ENOEXEC);
	}
	abi = dlsym(h, "bsdkern_module_abi");
	if (abi == NULL || *abi != NETD_MODULE_ABI) {
		printf("link_dl: %s: not a module of this server (ABI %d, want %d)\n",
		    filename, abi != NULL ? *abi : -1, NETD_MODULE_ABI);
		dlclose(h);
		return (EFTYPE);
	}
	f = (struct dl_file *)linker_make_file(filename, &link_dl_class);
	if (f == NULL) {
		dlclose(h);
		return (ENOMEM);
	}
	f->handle = h;
	/*
	 * Per-CPU variables of its own: room in each CPU's block, their
	 * initial values copied there, and the module told how far it is
	 * (its include/sys/pcpu.h adds that), before any of its code runs.
	 */
	if (defined_in(h, abi, "__start_set_pcpu")) {
		char *start = dlsym(h, "__start_set_pcpu");
		char *stop = dlsym(h, "__stop_set_pcpu");
		uintptr_t *delta = dlsym(h, "bsdkern_module_dpcpu_delta");
		size_t size = stop - start;
		uintptr_t at;
		int cpu;

		if (delta == NULL || (f->dpcpu = dpcpu_take(size)) == 0) {
			printf("link_dl: %s: no per-CPU room for %zu bytes\n", filename, size);
			linker_file_unload(&f->lf, LINKER_UNLOAD_FORCE);
			return (ENOSPC);
		}
		f->dpcpu_size = size;
		at = dpcpu_room() + f->dpcpu;
		CPU_FOREACH(cpu)
			memcpy((void *)(dpcpu_off[cpu] + at), start, size);
		*delta = at - (uintptr_t)start;
	}
	*result = &f->lf;
	return (0);
}

/* The class, and the program itself as the kernel's file. */
static void
link_dl_init(void *dummy)
{
	(void)dummy;
	linker_add_class(&link_dl_class);
	linker_kernel_file = linker_make_file(getprogname(), &link_dl_class);
	if (linker_kernel_file == NULL)
		panic("link_dl: no kernel file");
	linker_kernel_file->address = (caddr_t)&link_dl_init;
	linker_kernel_file->size = 0;
}
SYSINIT(link_dl, SI_SUB_KLD, SI_ORDER_SECOND, link_dl_init, NULL);

/* What kern_linker.c needs of the rest of a kernel. */
bool boottrace_enabled;
int
boottrace(const char *td, const char *fmt, ...)
{
	(void)td;
	(void)fmt;
	return (0);
}

/*
 * The vnode calls kern_linker.c makes to search the module path: netd has
 * no vnodes, so only absolute paths load (linker_search_kld() copies them),
 * and a search finds nothing. The root counts as mounted.
 */
#include <sys/vnode.h>
#include <sys/namei.h>
#include <sys/filedesc.h>

static struct vnode root_stand_in;
struct vnode *rootvnode = &root_stand_in;
struct vnodeop_desc vop_getattr_desc;

int
vn_open(struct nameidata *ndp, int *flagp, int cmode, struct file *fp)
{
	(void)ndp;
	(void)flagp;
	(void)cmode;
	(void)fp;
	return (ENOENT);
}

int
vn_close(struct vnode *vp, int flags, struct ucred *cred, struct thread *td)
{
	(void)vp;
	(void)flags;
	(void)cred;
	(void)td;
	return (0);
}

int
vn_rdwr(enum uio_rw rw, struct vnode *vp, void *base, int len, off_t offset,
    enum uio_seg segflg, int ioflg, struct ucred *active_cred,
    struct ucred *file_cred, ssize_t *aresid, struct thread *td)
{
	(void)rw; (void)vp; (void)base; (void)len; (void)offset; (void)segflg;
	(void)ioflg; (void)active_cred; (void)file_cred; (void)aresid; (void)td;
	return (ENOENT);
}

/* Its root directory is the root's stand-in (linker_root_mounted()). */
static struct pwd pwd_stand_in = { .pwd_rdir = &root_stand_in };

struct pwd *
pwd_hold(struct thread *td)
{
	(void)td;
	return (&pwd_stand_in);
}

void
pwd_drop(struct pwd *pwd)
{
	(void)pwd;
}

/* Only a preloaded file's SYSINITs come here, at boot: netd has none. */
void
sysinit_add(struct sysinit **set, struct sysinit **set_end)
{
	(void)set;
	(void)set_end;
	panic("sysinit_add: no preloaded modules here");
}

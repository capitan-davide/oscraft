/*
 * FreeBSD's kmem_* and vm_page_* for UMA, over src/mem.rs. An address is its
 * own physical address: with dmap_phys_base at DMAP_MIN_ADDRESS,
 * PHYS_TO_DMAP() and DMAP_TO_PHYS() change nothing, and UMA's direct-map
 * paths stay as they are.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/mutex.h>
#include <sys/proc.h>
#include <sys/vmmeter.h>
#include <vm/vm.h>
#include <vm/vm_param.h>
#include <vm/pmap.h>
#include <vm/vm_page.h>
#include <vm/vm_pagequeue.h>
#include <vm/vm_phys.h>
#include <vm/vm_extern.h>
#include <vm/vm_kern.h>

#include "bsdkern.h"

vm_paddr_t dmap_phys_base = DMAP_MIN_ADDRESS;
vm_paddr_t dmap_phys_max = ~(vm_paddr_t)0;

static vm_offset_t base;
static vm_offset_t table;

void
bsdkern_mem_start(void)
{
	bsdkern_mem_init(sizeof(struct vm_page));
	base = bsdkern_mem_base(&table);
}

vm_page_t
PHYS_TO_VM_PAGE(vm_paddr_t pa)
{
	return ((vm_page_t)table + atop(pa - base));
}

vm_paddr_t
pmap_kextract(vm_offset_t va)
{
	return (va);
}

vm_paddr_t
pmap_extract(pmap_t pmap, vm_offset_t va)
{
	(void)pmap;
	return (va);
}

static vm_offset_t
pages(vm_size_t size, u_long align, int flags)
{
	vm_size_t n = atop(round_page(size));
	vm_offset_t va;
	vm_size_t i;

	va = bsdkern_pages_alloc(n, MAX(atop(align), 1));
	if (va == 0)
		return (0);
	for (i = 0; i < n; i++)
		PHYS_TO_VM_PAGE(va + ptoa(i))->phys_addr = va + ptoa(i);
	if ((flags & M_ZERO) != 0)
		memset((void *)va, 0, ptoa(n));
	return (va);
}

void *
kmem_malloc_domainset(struct domainset *ds, vm_size_t size, int flags)
{
	(void)ds;
	return ((void *)pages(size, 0, flags));
}

void *
kmem_malloc(vm_size_t size, int flags)
{
	return ((void *)pages(size, 0, flags));
}

void *
kmem_alloc_contig_domainset(struct domainset *ds, vm_size_t size, int flags,
    vm_paddr_t low, vm_paddr_t high, u_long align, vm_paddr_t boundary,
    vm_memattr_t memattr)
{
	(void)ds;
	(void)low;
	(void)high;
	(void)boundary;
	(void)memattr;
	return ((void *)pages(size, align, flags));
}

void *
kmem_alloc_contig(vm_size_t size, int flags, vm_paddr_t low, vm_paddr_t high,
    u_long align, vm_paddr_t boundary, vm_memattr_t memattr)
{
	return (kmem_alloc_contig_domainset(NULL, size, flags, low, high, align,
	    boundary, memattr));
}

/*
 * contigmalloc() for a driver (a module's: mods/include/sys/malloc.h):
 * memory that is contiguous in physical addresses too, and below `high`,
 * for a device. The pages are the arena's, as contigmalloc()'s must be
 * (free() finds their slab by address), and the arena is backed by the
 * process manager a chunk at a time with no promise of contiguity: what
 * isn't contiguous is held aside while another piece is tried, then given
 * back. NULL after a few tries.
 */
uint64_t bsdkern_phys(const void *va, size_t *contig);
void	*bsdkern_contigmalloc(unsigned long, struct malloc_type *, int,
	    vm_paddr_t, vm_paddr_t, unsigned long, vm_paddr_t);

void *
bsdkern_contigmalloc(unsigned long size, struct malloc_type *type, int flags,
    vm_paddr_t low, vm_paddr_t high, unsigned long alignment,
    vm_paddr_t boundary)
{
	void *held[16], *p;
	size_t contig;
	uint64_t pa;
	int n = 0;

	for (;;) {
		p = contigmalloc(size, type, flags, low, high, alignment,
		    boundary);
		if (p == NULL)
			break;
		contig = 0;
		pa = bsdkern_phys(p, &contig);
		if (pa != 0 && contig >= size && pa >= low &&
		    pa + size - 1 <= high)
			break;
		if (n == nitems(held)) {
			free(p, type);
			p = NULL;
			break;
		}
		held[n++] = p;
	}
	while (n > 0)
		free(held[--n], type);
	if (p == NULL)
		printf("bsdkern: no %lu bytes of physically contiguous "
		    "memory\n", size);
	return (p);
}

void
kmem_free(void *addr, vm_size_t size)
{
	bsdkern_pages_free((vm_offset_t)addr, atop(round_page(size)));
}

vm_offset_t
kva_alloc(vm_size_t size)
{
	return (pages(size, 0, 0));
}

void
kva_free(vm_offset_t addr, vm_size_t size)
{
	bsdkern_pages_free(addr, atop(round_page(size)));
}

static vm_page_t
one_page(int req)
{
	vm_offset_t va = pages(PAGE_SIZE, 0,
	    (req & VM_ALLOC_ZERO) != 0 ? M_ZERO : 0);

	return (va != 0 ? PHYS_TO_VM_PAGE(va) : NULL);
}

vm_page_t
vm_page_alloc_noobj(int req)
{
	return (one_page(req));
}

vm_page_t
vm_page_alloc_noobj_domain(int domain, int req)
{
	(void)domain;
	return (one_page(req));
}

vm_page_t
vm_page_alloc_noobj_contig_domain(int domain, int req, u_long npages,
    vm_paddr_t low, vm_paddr_t high, u_long align, vm_paddr_t boundary,
    vm_memattr_t memattr)
{
	vm_offset_t va;

	(void)domain;
	(void)low;
	(void)high;
	(void)boundary;
	(void)memattr;
	va = pages(ptoa(npages), align, (req & VM_ALLOC_ZERO) != 0 ? M_ZERO : 0);
	return (va != 0 ? PHYS_TO_VM_PAGE(va) : NULL);
}

void
vm_page_free(vm_page_t m)
{
	bsdkern_pages_free(m->phys_addr, 1);
}

bool
vm_page_unwire_noq(vm_page_t m)
{
	(void)m;
	return (true);
}

/* UMA's boot pages: an address is already mapped. */
vm_offset_t
pmap_map(vm_offset_t *virt, vm_paddr_t start, vm_paddr_t end, int prot)
{
	(void)virt;
	(void)end;
	(void)prot;
	return (start);
}

extern void uma_startup1(vm_offset_t);
extern void uma_startup2(void);

/* vm_mem_init(), of which UMA is all that's needed. */
static void
vm_start(void *dummy)
{
	(void)dummy;
	vm_cnt.v_page_count = physmem;
	VM_DOMAIN(0)->vmd_page_count = physmem;
	uma_startup1(0);
	uma_startup2();
}
SYSINIT(oscraft_vm, SI_SUB_VM, SI_ORDER_FIRST, vm_start, NULL);

/*
 * UMA's per-CPU pages: kva_alloc() has already given real, contiguous
 * memory, so there is nothing to map. The pages UMA allocated to map there
 * stay unused: a leak, of a kind that happens only when a zone grows.
 */
void
pmap_qenter(vm_offset_t va, vm_page_t *m, int count)
{
	(void)va;
	(void)m;
	(void)count;
}

void
pmap_qremove(vm_offset_t va, int count)
{
	(void)va;
	(void)count;
}

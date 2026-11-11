/*
 * FreeBSD's busdma for a server holding drivers: its machine-independent
 * part (subr_bus_dma.c) and arm64's tags (busdma_machdep.c) as they are,
 * with this implementation where arm64 has its bounce one (busdma_bounce.c,
 * whose table bus_dma_bounce_impl the tags take). Memory comes from the
 * process manager, physically contiguous and uncached (dma.rs), so no
 * bouncing and no cache maintenance: a sync is a barrier. A buffer from
 * elsewhere is loaded by its physical pages (_MEM_OFFSET); cacheable
 * memory loaded so isn't kept coherent yet.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/memdesc.h>
#include <machine/bus.h>
#include <machine/bus_dma_impl.h>

#include "bsdkern.h"

void	*bsdkern_dma_alloc(size_t len, bool nocache, uint64_t lowaddr,
	    uint64_t *pa);
void	bsdkern_unmap(void *va, size_t len);
uint64_t bsdkern_phys(const void *va, size_t *contig);

struct bus_dma_tag {
	struct bus_dma_tag_common common;
	bus_dma_segment_t *segments;
};

struct bus_dmamap {
	void		*va;		/* memory of mem_alloc(), or NULL */
	size_t		len;
	struct memdesc	mem;
	bus_dmamap_callback_t *callback;
	void		*callback_arg;
};

static int
tag_create(bus_dma_tag_t parent, bus_size_t alignment, bus_addr_t boundary,
    bus_addr_t lowaddr, bus_addr_t highaddr, bus_size_t maxsize, int nsegments,
    bus_size_t maxsegsz, int flags, bus_dma_lock_t *lockfunc, void *lockfuncarg,
    bus_dma_tag_t *dmat)
{
	bus_dma_tag_t t;
	int error;

	error = common_bus_dma_tag_create(parent != NULL ? &parent->common : NULL,
	    alignment, boundary, lowaddr, highaddr, maxsize, nsegments, maxsegsz,
	    flags, lockfunc, lockfuncarg, sizeof(*t), (void **)&t);
	if (error != 0)
		return (error);
	t->segments = malloc(sizeof(bus_dma_segment_t) * MAX(nsegments, 1),
	    M_DEVBUF, M_WAITOK | M_ZERO);
	*dmat = t;
	return (0);
}

static int
tag_destroy(bus_dma_tag_t dmat)
{
	free(dmat->segments, M_DEVBUF);
	free(dmat, M_DEVBUF);
	return (0);
}

static int
tag_set_domain(bus_dma_tag_t dmat)
{
	(void)dmat;
	return (0);
}

/* Bus addresses are physical, not this process's: never identity. */
static bool
id_mapped(bus_dma_tag_t dmat, vm_paddr_t buf, bus_size_t len)
{
	(void)dmat;
	(void)buf;
	(void)len;
	return (false);
}

static int
map_create(bus_dma_tag_t dmat, int flags, bus_dmamap_t *mapp)
{
	(void)dmat;
	(void)flags;
	*mapp = malloc(sizeof(**mapp), M_DEVBUF, M_WAITOK | M_ZERO);
	return (0);
}

static int
map_destroy(bus_dma_tag_t dmat, bus_dmamap_t map)
{
	(void)dmat;
	free(map, M_DEVBUF);
	return (0);
}

static int
mem_alloc(bus_dma_tag_t dmat, void **vaddr, int flags, bus_dmamap_t *mapp)
{
	size_t len = round_page(MAX(dmat->common.maxsize, 1));
	uint64_t pa;
	void *va;

	if (dmat->common.alignment > PAGE_SIZE)
		return (EINVAL);
	va = bsdkern_dma_alloc(len, true, dmat->common.lowaddr, &pa);
	if (va == NULL)
		return (ENOMEM);
	if (pa + len - 1 > dmat->common.lowaddr && dmat->common.lowaddr < BUS_SPACE_MAXADDR) {
		bsdkern_unmap(va, len);
		return (ENOMEM);
	}
	map_create(dmat, flags, mapp);
	(*mapp)->va = va;
	(*mapp)->len = len;
	if (flags & BUS_DMA_ZERO)
		memset(va, 0, len);
	*vaddr = va;
	return (0);
}

static void
mem_free(bus_dma_tag_t dmat, void *vaddr, bus_dmamap_t map)
{
	(void)vaddr;
	bsdkern_unmap(map->va, map->len);
	map_destroy(dmat, map);
}

/* One more piece: joined to the last segment if it follows on. */
static bus_size_t
add_segment(bus_dma_tag_t dmat, bus_addr_t addr, bus_size_t len,
    bus_dma_segment_t *segs, int *segp)
{
	bus_addr_t bmask = ~(dmat->common.boundary - 1);
	int seg = *segp;

	if (dmat->common.boundary > 0) {
		bus_addr_t limit = roundup2(addr + 1, dmat->common.boundary);
		if (len > limit - addr)
			len = limit - addr;
	}
	if (seg >= 0 && segs[seg].ds_addr + segs[seg].ds_len == addr &&
	    segs[seg].ds_len + len <= dmat->common.maxsegsz &&
	    (dmat->common.boundary == 0 ||
	    (segs[seg].ds_addr & bmask) == (addr & bmask))) {
		segs[seg].ds_len += len;
		return (len);
	}
	if (++seg >= (int)dmat->common.nsegments)
		return (0);
	segs[seg].ds_addr = addr;
	segs[seg].ds_len = len;
	*segp = seg;
	return (len);
}

static int
load_phys(bus_dma_tag_t dmat, bus_dmamap_t map, vm_paddr_t buf,
    bus_size_t buflen, int flags, bus_dma_segment_t *segs, int *segp)
{
	bus_size_t sg;

	(void)map;
	(void)flags;
	if (segs == NULL)
		segs = dmat->segments;
	while (buflen > 0) {
		sg = add_segment(dmat, buf, MIN(buflen, dmat->common.maxsegsz), segs, segp);
		if (sg == 0)
			return (EFBIG);
		buf += sg;
		buflen -= sg;
	}
	return (0);
}

// ANCHOR: load_buffer
static int
load_buffer(bus_dma_tag_t dmat, bus_dmamap_t map, void *buf, bus_size_t buflen,
    struct pmap *pmap, int flags, bus_dma_segment_t *segs, int *segp)
{
	size_t contig;
	uint64_t pa;
	int error;

	(void)pmap;
	while (buflen > 0) {
		pa = bsdkern_phys(buf, &contig);
		if (pa == 0 || contig == 0)
			return (EINVAL);
		contig = MIN(contig, buflen);
		error = load_phys(dmat, map, pa, contig, flags, segs, segp);
		if (error != 0)
			return (error);
		buf = (char *)buf + contig;
		buflen -= contig;
	}
	return (0);
}
// ANCHOR_END: load_buffer

static int
load_ma(bus_dma_tag_t dmat, bus_dmamap_t map, struct vm_page **ma,
    bus_size_t tlen, int ma_offs, int flags, bus_dma_segment_t *segs, int *segp)
{
	(void)dmat; (void)map; (void)ma; (void)tlen; (void)ma_offs; (void)flags;
	(void)segs; (void)segp;
	return (EOPNOTSUPP);
}

static void
map_waitok(bus_dma_tag_t dmat, bus_dmamap_t map, struct memdesc *mem,
    bus_dmamap_callback_t *callback, void *callback_arg)
{
	(void)dmat;
	map->mem = *mem;
	map->callback = callback;
	map->callback_arg = callback_arg;
}

static bus_dma_segment_t *
map_complete(bus_dma_tag_t dmat, bus_dmamap_t map, bus_dma_segment_t *segs,
    int nsegs, int error)
{
	(void)map;
	(void)nsegs;
	(void)error;
	return (segs != NULL ? segs : dmat->segments);
}

static void
map_unload(bus_dma_tag_t dmat, bus_dmamap_t map)
{
	(void)dmat;
	(void)map;
}

/* Uncached memory: what was written is there once the barrier passes. */
static void
map_sync(bus_dma_tag_t dmat, bus_dmamap_t map, bus_dmasync_op_t op)
{
	(void)dmat;
	(void)map;
	(void)op;
	__asm __volatile("dsb sy" ::: "memory");
}

struct bus_dma_impl bus_dma_bounce_impl = {
	.tag_create = tag_create,
	.tag_destroy = tag_destroy,
	.tag_set_domain = tag_set_domain,
	.id_mapped = id_mapped,
	.map_create = map_create,
	.map_destroy = map_destroy,
	.mem_alloc = mem_alloc,
	.mem_free = mem_free,
	.load_ma = load_ma,
	.load_phys = load_phys,
	.load_buffer = load_buffer,
	.map_waitok = map_waitok,
	.map_complete = map_complete,
	.map_unload = map_unload,
	.map_sync = map_sync,
};

/* One memory domain: every range matches it (vm_phys.c's, for the tags). */
int	vm_phys_domain_match(int prefer, vm_paddr_t low, vm_paddr_t high);

int
vm_phys_domain_match(int prefer, vm_paddr_t low, vm_paddr_t high)
{
	(void)prefer;
	(void)low;
	(void)high;
	return (0);
}

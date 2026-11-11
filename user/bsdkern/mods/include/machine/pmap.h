/*
 * FreeBSD's <machine/pmap.h> for a module of a bsdkern server, first on its
 * include path. In the server itself an address is its own "physical"
 * address (kern/mem.c: UMA and the stack need no more), but a driver asks
 * vtophys() for the address its device will use (virtqueue.c, for a virtio
 * ring), and that is the real one: the process manager's _MEM_OFFSET
 * (bsdkern_phys(), src/dma.rs), as QNX's drivers call mem_offset(). The
 * memory is the server's own, backed when mapped and never moved.
 */
#include_next <machine/pmap.h>

#if defined(_KERNEL) && !defined(LOCORE)
uint64_t	bsdkern_phys(const void *, size_t *);
#undef vtophys
#define	vtophys(va)	((vm_paddr_t)bsdkern_phys((const void *)(va), NULL))
#endif

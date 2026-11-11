/*
 * FreeBSD's <sys/malloc.h> for a module of a bsdkern server, first on its
 * include path. A driver's contigmalloc() is memory its device reaches by
 * one physical address (virtqueue.c's ring): the server's own makes no
 * such promise (kern/mem.c: its pages are contiguous in the server's
 * addresses), so a module's goes through bsdkern_contigmalloc(), which
 * gives only memory that is physically contiguous too. free() as ever.
 */
#include_next <sys/malloc.h>

#if defined(_KERNEL) && !defined(_BSDKERN_MODULE_MALLOC_H_)
#define	_BSDKERN_MODULE_MALLOC_H_
void	*bsdkern_contigmalloc(unsigned long, struct malloc_type *, int,
	    vm_paddr_t, vm_paddr_t, unsigned long, vm_paddr_t);
#define	contigmalloc	bsdkern_contigmalloc
#endif

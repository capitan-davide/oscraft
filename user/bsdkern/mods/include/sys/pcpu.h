/*
 * FreeBSD's <sys/pcpu.h> for a module of a bsdkern server, first on its include path: a
 * module's own per-CPU variables (DPCPU_DEFINE) are copied by the server into
 * space of its per-CPU blocks (link_dl.c), at bsdkern_module_dpcpu_delta from
 * where the module has them. FreeBSD's loader relocates their symbols
 * there instead, which dlopen() can't; io-sock patches this header with an
 * index per module (its dynmod_index, read with REA). The module's own are
 * those inside its set_pcpu (its __start_set_pcpu: modules link with
 * -Bsymbolic); the program's, through DPCPU_DECLARE, are not moved.
 */
#include_next <sys/pcpu.h>

#if defined(_KERNEL) && !defined(LOCORE)
extern uintptr_t bsdkern_module_dpcpu_delta;
#undef _DPCPU_PTR
#define	_DPCPU_PTR(b, n)						\
    ((__typeof(DPCPU_NAME(n))*)((b) + (uintptr_t)&DPCPU_NAME(n) +	\
    ((uintptr_t)&DPCPU_NAME(n) - DPCPU_START < DPCPU_STOP - DPCPU_START ? \
    bsdkern_module_dpcpu_delta : 0)))
#endif

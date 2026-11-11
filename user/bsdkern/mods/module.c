/*
 * In every module of a bsdkern server (netd, pcid): the ABI it was built for (link_dl.c checks it), and
 * references to the linker sets kern_linker.c looks up, so that the linker
 * defines their __start_set_ and __stop_set_ symbols (weak: a module may
 * have no SYSINIT or sysctl).
 */
const int bsdkern_module_abi = 1;

/* Where the server put this module's per-CPU variables (include/sys/pcpu.h). */
unsigned long bsdkern_module_dpcpu_delta;

#define	SET(name)							\
	extern void *__start_set_##name __attribute__((weak));		\
	extern void *__stop_set_##name __attribute__((weak));
SET(modmetadata_set)
SET(sysinit_set)
SET(sysuninit_set)
SET(sysctl_set)
SET(pcpu)

void *const bsdkern_module_sets[] __attribute__((used)) = {
	&__start_set_modmetadata_set, &__stop_set_modmetadata_set,
	&__start_set_sysinit_set, &__stop_set_sysinit_set,
	&__start_set_sysuninit_set, &__stop_set_sysuninit_set,
	&__start_set_sysctl_set, &__stop_set_sysctl_set,
	&__start_set_pcpu, &__stop_set_pcpu,
};

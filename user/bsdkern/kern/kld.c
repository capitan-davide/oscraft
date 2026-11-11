/*
 * kldload(2) and its family, for a program serving them: the call's number
 * (FreeBSD's syscall number), two integer arguments and a buffer, handed to
 * FreeBSD's own sys_*() functions on the calling thread. copyin() and
 * copyout() are copies here, so the buffer is the user memory they see:
 * the path or module name, or the stat structure (its version filled in).
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/kernel.h>
#include <sys/linker.h>
#include <sys/module.h>
#include <sys/proc.h>
#include <sys/syscall.h>
#include <sys/sysproto.h>

#include "bsdkern.h"

int	bsdkern_kld(int number, int a0, int a1, char *buf, long *ret);

int
bsdkern_kld(int number, int a0, int a1, char *buf, long *ret)
{
	struct thread *td;
	int error;

	bsdkern_attach();
	td = curthread;
	td->td_retval[0] = 0;
	switch (number) {
	case SYS_kldload: {
		struct kldload_args a = { .file = buf };
		error = sys_kldload(td, &a);
		break;
	}
	case SYS_kldunload: {
		struct kldunload_args a = { .fileid = a0 };
		error = sys_kldunload(td, &a);
		break;
	}
	case SYS_kldunloadf: {
		struct kldunloadf_args a = { .fileid = a0, .flags = a1 };
		error = sys_kldunloadf(td, &a);
		break;
	}
	case SYS_kldfind: {
		struct kldfind_args a = { .file = buf };
		error = sys_kldfind(td, &a);
		break;
	}
	case SYS_kldnext: {
		struct kldnext_args a = { .fileid = a0 };
		error = sys_kldnext(td, &a);
		break;
	}
	case SYS_kldstat: {
		struct kldstat_args a = { .fileid = a0, .stat = (struct kld_file_stat *)buf };
		error = sys_kldstat(td, &a);
		break;
	}
	case SYS_kldfirstmod: {
		struct kldfirstmod_args a = { .fileid = a0 };
		error = sys_kldfirstmod(td, &a);
		break;
	}
	case SYS_modnext: {
		struct modnext_args a = { .modid = a0 };
		error = sys_modnext(td, &a);
		break;
	}
	case SYS_modfnext: {
		struct modfnext_args a = { .modid = a0 };
		error = sys_modfnext(td, &a);
		break;
	}
	case SYS_modstat: {
		struct modstat_args a = { .modid = a0, .stat = (struct module_stat *)buf };
		error = sys_modstat(td, &a);
		break;
	}
	case SYS_modfind: {
		struct modfind_args a = { .name = buf };
		error = sys_modfind(td, &a);
		break;
	}
	default:
		error = ENOSYS;
	}
	*ret = td->td_retval[0];
	return (error);
}

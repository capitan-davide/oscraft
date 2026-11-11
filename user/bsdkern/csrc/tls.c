/* The per-thread pointers of <machine/pcpu.h> under OSCRAFT_BSDKERN. */
struct thread;
struct pcpu;

__thread struct thread *bsdkern_curthread;
__thread struct pcpu *bsdkern_curpcpu;

void
bsdkern_set_curthread(struct thread *td)
{
	bsdkern_curthread = td;
}

void
bsdkern_set_curpcpu(struct pcpu *pc)
{
	bsdkern_curpcpu = pc;
}

struct pcpu *
bsdkern_get_curpcpu(void)
{
	return (bsdkern_curpcpu);
}

struct thread *
bsdkern_get_curthread(void)
{
	return (bsdkern_curthread);
}

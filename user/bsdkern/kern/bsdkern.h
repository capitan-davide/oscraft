/* What the C part of bsdkern shares with its Rust part. */
#ifndef _BSDKERN_H_
#define _BSDKERN_H_

/* A struct thread's own part, at td_emuldata. */
struct bsdkern_td {
	struct mtx	lock;		/* its td_lock when not sleeping */
	void		*parker;
	volatile u_int	inhibited;
	int		bound;		/* sched_bind() */
	void		*start;		/* RFSTOPPED: what sched_add() starts */
	char		name[MAXCOMLEN + 1];
};

void	*bsdkern_parker_new(void);
void	bsdkern_park(void *);
void	bsdkern_unpark(void *);
void	bsdkern_yield(void);
void	bsdkern_bind(int);
void	bsdkern_sel_wakeup(void *);
struct pcpu *bsdkern_slot_pcpu(int);
void	bsdkern_slot_attach(void);
void	bsdkern_mem_start(void);
void	bsdkern_attach(void);
/* Per-CPU room for modules, after the program's own (link_dl.c). */
#define	BSDKERN_DPCPU_MODSPACE	(64 * 1024)
void	bsdkern_vm_counters(void);
void	bsdkern_thread0(void);
void	bsdkern_sched_init(void);
void	bsdkern_pcpus_init(int);
void	bsdkern_slots_init(int);
void	bsdkern_clock_start(void);
void	bsdkern_nanosleep(uint64_t);
void	bsdkern_console(const char *, size_t);
void	bsdkern_panic(const char *) __dead2;
void	bsd_qsort(void *, size_t, size_t, int (*)(const void *, const void *));
void	arc4random_buf(void *, size_t);
void	bsdkern_mem_init(size_t);
u_long	bsdkern_mem_base(vm_offset_t *);
u_long	bsdkern_pages_alloc(size_t, size_t);
void	bsdkern_pages_free(u_long, size_t);
void	bsdkern_slot_pin_query(int (*)(void));
void	bsdkern_set_curthread(struct thread *);
void	*bsdkern_zalloc(size_t);
void	bsdkern_free(void *);
int	bsdkern_thread_spawn(void *, const char *);
void	bsdkern_thread_exit(void) __dead2;

#endif

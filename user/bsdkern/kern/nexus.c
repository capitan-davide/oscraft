/*
 * The root of the device tree in a server: FreeBSD's nexus, which gives its
 * children their resources. In the kernel those come from the firmware
 * (FDT, ACPI); here the server names its devices, with a register window
 * and an interrupt each (bsdkern_nexus_add(), from its options, as QNX's
 * drivers take "mem,irq" on their command lines), and the nexus serves:
 *
 * - memory: the window, through arm64's own bus space (bus_machdep.c,
 *   unchanged) over pmap_mapdev_attr(), which maps the registers uncached
 *   with the process manager's _MEM_MAP (bsdkern_map_device());
 * - interrupts: a thread per handler, attached to the interrupt with
 *   InterruptAttachThread() and waiting in InterruptWait(), which unmasks
 *   it each time (the kernel masks it on delivery): the filter runs there,
 *   then, if the filter asks or there is none, the handler, as FreeBSD's
 *   interrupt thread would; as a QNX driver's interrupt thread does.
 */
#include "opt_platform.h"

#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/malloc.h>
#include <sys/module.h>
#include <sys/proc.h>
#include <sys/rman.h>

#include <vm/vm.h>
#include <vm/pmap.h>

#include <machine/bus.h>
#include <machine/resource.h>

#ifdef FDT
#include <dev/ofw/openfirm.h>

#include "ofw_bus_if.h"

u_int	intr_fdt_map_irq(phandle_t, pcell_t *, u_int);

/* A device tree's interrupt specifier: the system page's vector (fdt.c). */
static int
nexus_ofw_map_intr(device_t dev, device_t child, phandle_t iparent,
    int icells, pcell_t *intr)
{
	return (intr_fdt_map_irq(iparent, intr, icells));
}
#endif

void	*bsdkern_map_device(uint64_t, size_t);
void	 bsdkern_unmap(void *, size_t);
int	 InterruptAttachThread_r(int, unsigned);
int	 InterruptWait_r(int, const uint64_t *);

/* InterruptWait(): unmask what the kernel masked on delivery. */
#define	INTR_WAIT_FLAGS_UNMASK	0x2

extern struct bus_space memmap_bus;
/* fdt.c's, in servers built with FDT: the board's tree was installed. */
bool	bsdkern_fdt_present(void) __weak_symbol;

static MALLOC_DEFINE(M_NEXUS, "nexus", "bsdkern's nexus");

void *
pmap_mapdev_attr(vm_paddr_t pa, vm_size_t size, vm_memattr_t ma)
{
	vm_offset_t off;
	char *va;

	(void)ma;	/* Device memory, posted or not: uncached either way. */
	off = pa & PAGE_MASK;
	va = bsdkern_map_device(pa - off, round_page(size + off));
	return (va == NULL ? NULL : va + off);
}

void *
pmap_mapdev(vm_paddr_t pa, vm_size_t size)
{
	return (pmap_mapdev_attr(pa, size, VM_MEMATTR_DEVICE));
}

void
pmap_unmapdev(void *p, vm_size_t size)
{
	vm_offset_t off;

	off = (vm_offset_t)p & PAGE_MASK;
	bsdkern_unmap((char *)p - off, round_page(size + off));
}

struct nexus_ivars {
	struct resource_list rl;
};

static struct rman mem_rman = {
	.rm_type = RMAN_ARRAY, .rm_descr = "I/O memory addresses",
};
static struct rman irq_rman = {
	.rm_type = RMAN_ARRAY, .rm_descr = "Interrupts",
};
static device_t nexus_dev;

static int
nexus_probe(device_t dev)
{
	device_quiet(dev);
	return (BUS_PROBE_GENERIC);
}

static int
nexus_attach(device_t dev)
{
	if (rman_init(&mem_rman) != 0 ||
	    rman_manage_region(&mem_rman, 0, BUS_SPACE_MAXADDR) != 0 ||
	    rman_init(&irq_rman) != 0 ||
	    rman_manage_region(&irq_rman, 0, ~0) != 0)
		panic("nexus: rman");
	nexus_dev = dev;
	/* A device tree's root bus, as the kernel's nexus_fdt_attach(). */
	if (bsdkern_fdt_present != NULL && bsdkern_fdt_present())
		BUS_ADD_CHILD(dev, 10, "ofwbus", 0);
	bus_attach_children(dev);
	return (0);
}

static device_t
nexus_add_child(device_t bus, u_int order, const char *name, int unit)
{
	struct nexus_ivars *iv;
	device_t child;

	iv = malloc(sizeof(*iv), M_NEXUS, M_WAITOK | M_ZERO);
	resource_list_init(&iv->rl);
	child = device_add_child_ordered(bus, order, name, unit);
	device_set_ivars(child, iv);
	return (child);
}

static struct resource_list *
nexus_get_reslist(device_t dev, device_t child)
{
	struct nexus_ivars *iv = device_get_ivars(child);

	return (&iv->rl);
}

static struct rman *
nexus_get_rman(device_t bus, int type, u_int flags)
{
	switch (type) {
	case SYS_RES_MEMORY:
		return (&mem_rman);
	case SYS_RES_IRQ:
		return (&irq_rman);
	default:
		return (NULL);
	}
}

static struct resource *
nexus_alloc_resource(device_t bus, device_t child, int type, int *rid,
    rman_res_t start, rman_res_t end, rman_res_t count, u_int flags)
{
	struct resource_list_entry *rle;

	/* The defaults: what the server named for the child. */
	if (RMAN_IS_DEFAULT_RANGE(start, end) &&
	    device_get_parent(child) == bus) {
		rle = resource_list_find(BUS_GET_RESOURCE_LIST(bus, child),
		    type, *rid);
		if (rle == NULL)
			return (NULL);
		start = rle->start;
		end = rle->end;
		count = rle->count;
	}
	return (bus_generic_rman_alloc_resource(bus, child, type, rid, start,
	    end, count, flags));
}

/*
 * An interrupt is the rman's entry alone (without INTRNG's mapping): the
 * kernel attaches it when a handler is set up.
 */
static int
nexus_activate_resource(device_t bus, device_t child, struct resource *r)
{
	if (rman_get_type(r) == SYS_RES_IRQ)
		return (rman_activate_resource(r));
	return (bus_generic_rman_activate_resource(bus, child, r));
}

static int
nexus_deactivate_resource(device_t bus, device_t child, struct resource *r)
{
	if (rman_get_type(r) == SYS_RES_IRQ)
		return (rman_deactivate_resource(r));
	return (bus_generic_rman_deactivate_resource(bus, child, r));
}

static int
nexus_map_resource(device_t bus, device_t child, struct resource *r,
    struct resource_map_request *argsp, struct resource_map *map)
{
	struct resource_map_request args;
	rman_res_t length, start;
	bus_space_handle_t h;
	int error;

	if (rman_get_type(r) != SYS_RES_MEMORY)
		return (EINVAL);
	resource_init_map_request(&args);
	error = resource_validate_map_request(r, argsp, &args, &start, &length);
	if (error != 0)
		return (error);
	error = bus_space_map(&memmap_bus, start, length, 0, &h);
	if (error != 0)
		return (error);
	map->r_bustag = &memmap_bus;
	map->r_bushandle = h;
	map->r_vaddr = (void *)h;
	map->r_size = length;
	return (0);
}

static int
nexus_unmap_resource(device_t bus, device_t child, struct resource *r,
    struct resource_map *map)
{
	if (rman_get_type(r) != SYS_RES_MEMORY)
		return (EINVAL);
	bus_space_unmap(map->r_bustag, map->r_bushandle, map->r_size);
	return (0);
}

/* An interrupt handler and the thread that runs it. */
struct nexus_intr {
	int		 irq;
	driver_filter_t	*filter;
	driver_intr_t	*handler;
	void		*arg;
	volatile int	 state;		/* 0 starting, 1 attached, -1 failed */
	volatile int	 gone;
};

// ANCHOR: interrupt_thread
// ANCHOR: nexus_ithread
static void
nexus_ithread(void *a)
{
	struct nexus_intr *ih = a;
	int r;

	r = InterruptAttachThread_r(ih->irq, 0);
	if (r < 0) {
		printf("nexus: irq %d: InterruptAttachThread: %d\n", ih->irq, -r);
		ih->state = -1;
		return;
	}
	ih->state = 1;
	/* Until torn down; the thread ends at the interrupt after that. */
	while (!ih->gone) {
		r = InterruptWait_r(INTR_WAIT_FLAGS_UNMASK, NULL);
		if (r != 0 || ih->gone)
			continue;
		if (ih->filter == NULL ||
		    (ih->filter(ih->arg) & FILTER_SCHEDULE_THREAD) != 0) {
			if (ih->handler != NULL)
				ih->handler(ih->arg);
		}
	}
// ANCHOR_END: nexus_ithread
}
// ANCHOR_END: interrupt_thread

static int
nexus_setup_intr(device_t bus, device_t child, struct resource *irq,
    int flags, driver_filter_t *filter, driver_intr_t *handler, void *arg,
    void **cookiep)
{
	struct nexus_intr *ih;
	int error;

	if (rman_get_type(irq) != SYS_RES_IRQ)
		return (EINVAL);
	ih = malloc(sizeof(*ih), M_NEXUS, M_WAITOK | M_ZERO);
	ih->irq = rman_get_start(irq);
	ih->filter = filter;
	ih->handler = handler;
	ih->arg = arg;
	error = kthread_add(nexus_ithread, ih, NULL, NULL, 0, 0, "irq%d: %s",
	    ih->irq, device_get_nameunit(child));
	if (error != 0) {
		free(ih, M_NEXUS);
		return (error);
	}
	while (ih->state == 0)
		pause("nxintr", 1);
	if (ih->state < 0) {
		/* The thread has ended; the record is left to it. */
		return (ENXIO);
	}
	*cookiep = ih;
	return (0);
}

static int
nexus_teardown_intr(device_t bus, device_t child, struct resource *irq,
    void *cookie)
{
	struct nexus_intr *ih = cookie;

	ih->gone = 1;
	return (0);
}

static device_method_t nexus_methods[] = {
	DEVMETHOD(device_probe,		nexus_probe),
	DEVMETHOD(device_attach,	nexus_attach),
	DEVMETHOD(device_detach,	bus_generic_detach),
	DEVMETHOD(device_shutdown,	bus_generic_shutdown),

	DEVMETHOD(bus_add_child,	nexus_add_child),
	DEVMETHOD(bus_print_child,	bus_generic_print_child),
	DEVMETHOD(bus_get_resource_list, nexus_get_reslist),
	DEVMETHOD(bus_get_resource,	bus_generic_rl_get_resource),
	DEVMETHOD(bus_set_resource,	bus_generic_rl_set_resource),
	DEVMETHOD(bus_delete_resource,	bus_generic_rl_delete_resource),
	DEVMETHOD(bus_get_rman,		nexus_get_rman),
	DEVMETHOD(bus_alloc_resource,	nexus_alloc_resource),
	DEVMETHOD(bus_activate_resource, nexus_activate_resource),
	DEVMETHOD(bus_deactivate_resource, nexus_deactivate_resource),
	DEVMETHOD(bus_adjust_resource,	bus_generic_rman_adjust_resource),
	DEVMETHOD(bus_release_resource,	bus_generic_rman_release_resource),
	DEVMETHOD(bus_map_resource,	nexus_map_resource),
	DEVMETHOD(bus_unmap_resource,	nexus_unmap_resource),
	DEVMETHOD(bus_setup_intr,	nexus_setup_intr),
	DEVMETHOD(bus_teardown_intr,	nexus_teardown_intr),
#ifdef FDT
	DEVMETHOD(ofw_bus_map_intr,	nexus_ofw_map_intr),
#endif

	DEVMETHOD_END
};

static driver_t nexus_driver = { "nexus", nexus_methods, 1 };

/* With -v: the device tree, as devinfo(8) would show it. */
static void
nexus_print_tree(device_t dev, int depth)
{
	device_t *kids;
	int i, n;

	printf("%*s%s%s\n", 2 * depth, "", device_get_nameunit(dev),
	    device_is_attached(dev) ? "" : " (not attached)");
	if (device_get_children(dev, &kids, &n) != 0)
		return;
	for (i = 0; i < n; i++)
		nexus_print_tree(kids[i], depth + 1);
	free(kids, M_TEMP);
}

/*
 * The nexus under root, as in the kernel; then the kernel's
 * root_bus_configure() (autoconf), which raises the bus pass to the
 * default, as newbus probes only drivers of passes reached so far.
 */
static void
nexus_start(void *arg)
{
	device_t dev;

	(void)arg;
	bus_topo_lock();
	dev = device_add_child(root_bus, "nexus", 0);
	if (dev == NULL)
		panic("nexus: not added");
	root_bus_configure();
	if (!device_is_attached(dev))
		panic("nexus: not attached");
	if (bootverbose)
		nexus_print_tree(root_bus, 0);
	bus_topo_unlock();
}
SYSINIT(bsdkern_nexus, SI_SUB_CONFIGURE, SI_ORDER_FIRST, nexus_start, NULL);
EARLY_DRIVER_MODULE(nexus, root, nexus_driver, 0, 0, BUS_PASS_BUS);

/*
 * A device the server names: driver `name`, registers at physical `mem`,
 * `size` bytes (rid 0), interrupt `irq` (rid 0; -1 for none); probed and
 * attached at once. 0, or an errno.
 */
int
bsdkern_nexus_add(const char *name, uint64_t mem, uint64_t size, int irq)
{
	device_t child;
	int error;

	if (nexus_dev == NULL)
		return (ENXIO);
	bus_topo_lock();
	child = BUS_ADD_CHILD(nexus_dev, 0, name, DEVICE_UNIT_ANY);
	if (child == NULL) {
		bus_topo_unlock();
		return (ENOMEM);
	}
	if (size != 0)
		bus_set_resource(child, SYS_RES_MEMORY, 0, mem, size);
	if (irq >= 0)
		bus_set_resource(child, SYS_RES_IRQ, 0, irq, 1);
	error = device_probe_and_attach(child);
	if (error != 0)
		device_delete_child(nexus_dev, child);
	bus_topo_unlock();
	return (error);
}

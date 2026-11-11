/*
 * A PCI bus in a server holding drivers (usbd: xhci), over pcid, which
 * owns the real one (user/pcid): FreeBSD's PCI drivers attach to a bus
 * named "pci" here as in the kernel (DRIVER_MODULE(xhci, pci, ...)), and
 * what they ask of it goes to pcid through /dev/pci, in FreeBSD's own
 * requests:
 *
 * - the devices: PCIOCGETCONF, each one's struct pci_conf as its ivars
 *   (vendor, device, class, ...);
 * - configuration space: PCIOCREAD, PCIOCWRITE; pcid alone touches the
 *   host bridge, and its lock;
 * - windows and the legacy interrupt: PCIOCROUTE (bsdkern_pci.h): a BAR's
 *   CPU address and the INTx vector, which this server then maps and
 *   attaches itself on its nexus: the data never passes through pcid.
 *
 * MSI is not offered yet (alloc_msi fails), so drivers use INTx.
 * Built in servers whose configuration asks for it (BSDKERN_PCI_CLIENT:
 * usbd), or as a module of a server that takes its drivers as modules
 * (BSDKERN_PCI_CLIENT_MODULE: netd's pci.ko, before virtio_pci.ko); pcid
 * has the real bus.
 */
#ifdef BSDKERN_PCI_CLIENT
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/kernel.h>
#include <sys/malloc.h>
#include <sys/module.h>
#include <sys/rman.h>

#include <machine/bus.h>
#include <machine/resource.h>

#include <dev/pci/pcireg.h>
#include <dev/pci/pcivar.h>

#include "pci_if.h"
#include "pcib_if.h"
#include "bsdkern_pci.h"

/* libc's, in this process. */
int	open(const char *, int, ...);
int	ioctl(int, unsigned long, ...);
#define	O_RDWR	0x0002

static MALLOC_DEFINE(M_PCICL, "pciclient", "the PCI client bus");

struct pcicl_dev {
	struct pci_conf		conf;
	struct resource_list	rl;
};

static int pci_fd = -1;
static device_t pcicl_bus;

static int
pcicl_probe(device_t dev)
{
	device_set_desc(dev, "PCI bus, through pcid");
	return (BUS_PROBE_GENERIC);
}

static int
pcicl_attach(device_t dev)
{
	struct pci_conf_io pc;
	struct pci_conf *confs;
	struct pcicl_dev *d;
	device_t child;
	int i, n = 64;

	pci_fd = open("/dev/pci", O_RDWR);
	if (pci_fd < 0) {
		device_printf(dev, "no /dev/pci: is pcid running?\n");
		return (ENXIO);
	}
	confs = malloc(n * sizeof(*confs), M_PCICL, M_WAITOK | M_ZERO);
	bzero(&pc, sizeof(pc));
	pc.match_buf_len = n * sizeof(*confs);
	pc.matches = confs;
	if (ioctl(pci_fd, PCIOCGETCONF, &pc) != 0 ||
	    pc.status == PCI_GETCONF_ERROR) {
		device_printf(dev, "PCIOCGETCONF failed\n");
		free(confs, M_PCICL);
		return (ENXIO);
	}
	pcicl_bus = dev;
	for (i = 0; i < (int)pc.num_matches; i++) {
		/* Bridges are pcid's. */
		if (confs[i].pc_hdr != PCIM_HDRTYPE_NORMAL)
			continue;
		d = malloc(sizeof(*d), M_PCICL, M_WAITOK | M_ZERO);
		d->conf = confs[i];
		resource_list_init(&d->rl);
		child = device_add_child(dev, NULL, DEVICE_UNIT_ANY);
		device_set_ivars(child, d);
	}
	free(confs, M_PCICL);
	bus_attach_children(dev);
	return (0);
}

static int
pcicl_read_ivar(device_t dev, device_t child, int which, uintptr_t *result)
{
	struct pcicl_dev *d = device_get_ivars(child);
	struct pci_conf *c = &d->conf;

	switch (which) {
	case PCI_IVAR_SUBVENDOR:	*result = c->pc_subvendor; break;
	case PCI_IVAR_SUBDEVICE:	*result = c->pc_subdevice; break;
	case PCI_IVAR_VENDOR:		*result = c->pc_vendor; break;
	case PCI_IVAR_DEVICE:		*result = c->pc_device; break;
	case PCI_IVAR_DEVID:
		*result = c->pc_device << 16 | c->pc_vendor;
		break;
	case PCI_IVAR_CLASS:		*result = c->pc_class; break;
	case PCI_IVAR_SUBCLASS:		*result = c->pc_subclass; break;
	case PCI_IVAR_PROGIF:		*result = c->pc_progif; break;
	case PCI_IVAR_REVID:		*result = c->pc_revid; break;
	case PCI_IVAR_DOMAIN:		*result = c->pc_sel.pc_domain; break;
	case PCI_IVAR_BUS:		*result = c->pc_sel.pc_bus; break;
	case PCI_IVAR_SLOT:		*result = c->pc_sel.pc_dev; break;
	case PCI_IVAR_FUNCTION:		*result = c->pc_sel.pc_func; break;
	case PCI_IVAR_CMDREG:
		*result = PCI_READ_CONFIG(dev, child, PCIR_COMMAND, 2);
		break;
	case PCI_IVAR_IRQ:		*result = 255; break;
	case PCI_IVAR_INTPIN:
		*result = PCI_READ_CONFIG(dev, child, PCIR_INTPIN, 1);
		break;
	default:
		return (ENOENT);
	}
	return (0);
}

static uint32_t
pcicl_read_config(device_t dev, device_t child, int reg, int width)
{
	struct pcicl_dev *d = device_get_ivars(child);
	struct pci_io io;

	bzero(&io, sizeof(io));
	io.pi_sel = d->conf.pc_sel;
	io.pi_reg = reg;
	io.pi_width = width;
	if (ioctl(pci_fd, PCIOCREAD, &io) != 0)
		return (~0u);
	return (io.pi_data);
}

static void
pcicl_write_config(device_t dev, device_t child, int reg, uint32_t val,
    int width)
{
	struct pcicl_dev *d = device_get_ivars(child);
	struct pci_io io;

	bzero(&io, sizeof(io));
	io.pi_sel = d->conf.pc_sel;
	io.pi_reg = reg;
	io.pi_width = width;
	io.pi_data = val;
	(void)ioctl(pci_fd, PCIOCWRITE, &io);
}

static int
pcicl_command(device_t dev, device_t child, uint16_t set, uint16_t clear)
{
	uint16_t cmd;

	cmd = PCI_READ_CONFIG(dev, child, PCIR_COMMAND, 2);
	cmd = (cmd | set) & ~clear;
	PCI_WRITE_CONFIG(dev, child, PCIR_COMMAND, cmd, 2);
	return (0);
}

static int
pcicl_enable_busmaster(device_t dev, device_t child)
{
	return (pcicl_command(dev, child, PCIM_CMD_BUSMASTEREN, 0));
}

static int
pcicl_disable_busmaster(device_t dev, device_t child)
{
	return (pcicl_command(dev, child, 0, PCIM_CMD_BUSMASTEREN));
}

static int
pcicl_enable_io(device_t dev, device_t child, int space)
{
	return (pcicl_command(dev, child, space == SYS_RES_MEMORY ?
	    PCIM_CMD_MEMEN : PCIM_CMD_PORTEN, 0));
}

static int
pcicl_disable_io(device_t dev, device_t child, int space)
{
	return (pcicl_command(dev, child, 0, space == SYS_RES_MEMORY ?
	    PCIM_CMD_MEMEN : PCIM_CMD_PORTEN));
}

/*
 * The capability list in configuration space, as pci.c walks it: from the
 * capability at `ptr` on, the first one of type `cap`.
 */
static int
pcicl_walk_caps(device_t dev, device_t child, int cap, int ptr, int *capreg)
{
	int n;

	for (n = 0; ptr != 0 && n < 48; n++) {
		if (PCI_READ_CONFIG(dev, child, ptr + PCICAP_ID, 1) == cap) {
			if (capreg != NULL)
				*capreg = ptr;
			return (0);
		}
		ptr = PCI_READ_CONFIG(dev, child, ptr + PCICAP_NEXTPTR, 1);
	}
	return (ENOENT);
}

/* The next one after the capability at `start` (pci_find_next_cap()). */
static int
pcicl_find_next_cap(device_t dev, device_t child, int cap, int start,
    int *capreg)
{
	return (pcicl_walk_caps(dev, child, cap,
	    PCI_READ_CONFIG(dev, child, start + PCICAP_NEXTPTR, 1), capreg));
}

static int
pcicl_find_cap(device_t dev, device_t child, int cap, int *capreg)
{
	if ((PCI_READ_CONFIG(dev, child, PCIR_STATUS, 2) &
	    PCIM_STATUS_CAPPRESENT) == 0)
		return (ENXIO);
	return (pcicl_walk_caps(dev, child, cap,
	    PCI_READ_CONFIG(dev, child, PCIR_CAP_PTR, 1), capreg));
}

static int
pcicl_find_extcap(device_t dev, device_t child, int cap, int *capreg)
{
	return (ENOENT);
}

static int
pcicl_no_msi(device_t dev, device_t child, int *count)
{
	return (ENXIO);
}

static int
pcicl_release_msi(device_t dev, device_t child)
{
	return (ENODEV);
}

static int
pcicl_zero(device_t dev, device_t child)
{
	return (0);
}

static int
pcicl_minus_one(device_t dev, device_t child)
{
	return (-1);
}

static int
pcicl_get_powerstate(device_t dev, device_t child)
{
	return (PCI_POWERSTATE_D0);
}

static int
pcicl_set_powerstate(device_t dev, device_t child, int state)
{
	return (0);
}

static int
pcicl_get_id(device_t dev, device_t child, enum pci_id_type type,
    uintptr_t *id)
{
	struct pcicl_dev *d = device_get_ivars(child);

	if (type != PCI_ID_RID)
		return (ENXIO);
	*id = PCI_RID(d->conf.pc_sel.pc_bus, d->conf.pc_sel.pc_dev,
	    d->conf.pc_sel.pc_func);
	return (0);
}

static int
pcicl_route(struct pcicl_dev *d, uint32_t reg, struct bsdkern_pci_route *r)
{
	bzero(r, sizeof(*r));
	r->sel = d->conf.pc_sel;
	r->reg = reg;
	return (ioctl(pci_fd, PCIOCROUTE, r) != 0 ? ENXIO : 0);
}

// ANCHOR: pcicl_alloc_resource
static struct resource *
pcicl_alloc_resource(device_t dev, device_t child, int type, int *rid,
    rman_res_t start, rman_res_t end, rman_res_t count, u_int flags)
{
	struct pcicl_dev *d;
	struct bsdkern_pci_route r;
	struct resource *res;

	if (device_get_parent(child) != dev)
		return (bus_generic_alloc_resource(dev, child, type, rid,
		    start, end, count, flags));
	d = device_get_ivars(child);
	switch (type) {
	case SYS_RES_MEMORY:
		if (pcicl_route(d, *rid, &r) != 0 || r.len == 0)
			return (NULL);
		start = r.base;
		count = r.len;
		break;
	case SYS_RES_IRQ:
		/* rid 0: INTx; MSI's rids are not offered. */
		if (*rid != 0 || pcicl_route(d, 0, &r) != 0 || r.irq < 0)
			return (NULL);
		start = r.irq;
		count = 1;
		break;
	default:
		return (NULL);
	}
	end = start + count - 1;
	/* On the nexus, as this server's own: it maps and attaches it. */
	res = BUS_ALLOC_RESOURCE(device_get_parent(dev), child, type, rid,
	    start, end, count, flags);
	if (res != NULL && type == SYS_RES_MEMORY)
		pcicl_enable_io(dev, child, SYS_RES_MEMORY);
	return (res);
// ANCHOR_END: pcicl_alloc_resource
}

static int
pcicl_release_resource(device_t dev, device_t child, struct resource *r)
{
	return (BUS_RELEASE_RESOURCE(device_get_parent(dev), child, r));
}

static int
pcicl_print_child(device_t dev, device_t child)
{
	struct pcicl_dev *d = device_get_ivars(child);
	int n;

	n = bus_print_child_header(dev, child);
	n += printf(" at device %d.%d", d->conf.pc_sel.pc_dev,
	    d->conf.pc_sel.pc_func);
	n += bus_print_child_footer(dev, child);
	return (n);
}

static void
pcicl_probe_nomatch(device_t dev, device_t child)
{
	struct pcicl_dev *d = device_get_ivars(child);

	device_printf(dev, "<vendor 0x%04x, device 0x%04x, class %02x%02x%02x> "
	    "at device %d.%d (no driver attached)\n", d->conf.pc_vendor,
	    d->conf.pc_device, d->conf.pc_class, d->conf.pc_subclass,
	    d->conf.pc_progif, d->conf.pc_sel.pc_dev, d->conf.pc_sel.pc_func);
}

static device_method_t pcicl_methods[] = {
	DEVMETHOD(device_probe,		pcicl_probe),
	DEVMETHOD(device_attach,	pcicl_attach),
	DEVMETHOD(device_detach,	bus_generic_detach),
	DEVMETHOD(device_shutdown,	bus_generic_shutdown),

	DEVMETHOD(bus_print_child,	pcicl_print_child),
	DEVMETHOD(bus_probe_nomatch,	pcicl_probe_nomatch),
	DEVMETHOD(bus_read_ivar,	pcicl_read_ivar),
	DEVMETHOD(bus_alloc_resource,	pcicl_alloc_resource),
	DEVMETHOD(bus_release_resource,	pcicl_release_resource),
	DEVMETHOD(bus_activate_resource, bus_generic_activate_resource),
	DEVMETHOD(bus_deactivate_resource, bus_generic_deactivate_resource),
	DEVMETHOD(bus_setup_intr,	bus_generic_setup_intr),
	DEVMETHOD(bus_teardown_intr,	bus_generic_teardown_intr),
	DEVMETHOD(bus_get_dma_tag,	bus_generic_get_dma_tag),

	DEVMETHOD(pci_read_config,	pcicl_read_config),
	DEVMETHOD(pci_write_config,	pcicl_write_config),
	DEVMETHOD(pci_enable_busmaster,	pcicl_enable_busmaster),
	DEVMETHOD(pci_disable_busmaster, pcicl_disable_busmaster),
	DEVMETHOD(pci_enable_io,	pcicl_enable_io),
	DEVMETHOD(pci_disable_io,	pcicl_disable_io),
	DEVMETHOD(pci_get_powerstate,	pcicl_get_powerstate),
	DEVMETHOD(pci_set_powerstate,	pcicl_set_powerstate),
	DEVMETHOD(pci_find_cap,		pcicl_find_cap),
	DEVMETHOD(pci_find_next_cap,	pcicl_find_next_cap),
	DEVMETHOD(pci_find_extcap,	pcicl_find_extcap),
	DEVMETHOD(pci_alloc_msi,	pcicl_no_msi),
	DEVMETHOD(pci_alloc_msix,	pcicl_no_msi),
	DEVMETHOD(pci_release_msi,	pcicl_release_msi),
	DEVMETHOD(pci_msi_count,	pcicl_zero),
	DEVMETHOD(pci_msix_count,	pcicl_zero),
	DEVMETHOD(pci_msix_pba_bar,	pcicl_minus_one),
	DEVMETHOD(pci_msix_table_bar,	pcicl_minus_one),
	DEVMETHOD(pci_get_id,		pcicl_get_id),

	DEVMETHOD_END
};

static driver_t pcicl_driver = { "pci", pcicl_methods, 1 };
DRIVER_MODULE(pci, nexus, pcicl_driver, 0, 0);
/* What FreeBSD's PCI drivers depend on (MODULE_DEPEND(..., pci, 1, 1, 1)). */
MODULE_VERSION(pci, 1);

/* The server asks for the bus (usbd's "pci" option): 0, or an errno. */
int	bsdkern_pciclient_start(void);

int
bsdkern_pciclient_start(void)
{
	extern int bsdkern_nexus_add(const char *, uint64_t, uint64_t, int);

	return (bsdkern_nexus_add("pci", 0, 0, -1));
}

#ifdef BSDKERN_PCI_CLIENT_MODULE
/* As a module: the bus comes up when it is loaded, after its driver. */
static void
pcicl_start(void *arg)
{
	int error;

	(void)arg;
	error = bsdkern_pciclient_start();
	if (error != 0)
		printf("pci: the client bus is not attached: %d\n", error);
}
SYSINIT(pcicl, SI_SUB_DRIVERS, SI_ORDER_ANY, pcicl_start, NULL);
#endif
#endif

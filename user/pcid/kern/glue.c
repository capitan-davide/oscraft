/*
 * pcid's own glue to FreeBSD's PCI code: PCIOCGETCONF (pciconf -l), whose
 * struct pci_conf_io points to the client's pattern and match buffers.
 * libsys sends them packed after it, [pci_conf_io][patterns][room for
 * the matches], and gets the same back; here the pointers are made to
 * point into the packed buffer for pci_user.c, whose copyin() and
 * copyout() are copies in this process.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/conf.h>
#include <sys/pciio.h>
#include <sys/bus.h>
#include <sys/rman.h>

#include <dev/pci/pcireg.h>
#include <dev/pci/pcivar.h>

#include "bsdkern_pci.h"

struct bsdkern_open;
int	bsdkern_cdev_ioctl(struct bsdkern_open *, u_long, void *);
int	bsdkern_cdev_ioctl_packed(struct bsdkern_open *, u_long, void *,
	    size_t, size_t *);

/*
 * PCIOCROUTE (bsdkern_pci.h): a BAR's window as reserved by the PCI bus
 * (pci_add_resources(), or here for an unassigned one), translated to the CPU's addresses by the host
 * bridge (bus_translate_resource()), and the INTx interrupt the bridge
 * routed (pci_assign_interrupt(), rid 0).
 */
// ANCHOR: pci_route
static int
route(struct bsdkern_pci_route *r)
{
	struct resource_list_entry *rle;
	struct pci_devinfo *dinfo;
	rman_res_t cpu;
	device_t dev;
	int error;

	bus_topo_lock();
	dev = pci_find_dbsf(r->sel.pc_domain, r->sel.pc_bus, r->sel.pc_dev,
	    r->sel.pc_func);
	if (dev == NULL) {
		bus_topo_unlock();
		return (ENODEV);
	}
	dinfo = device_get_ivars(dev);
	r->irq = -1;
	rle = resource_list_find(&dinfo->resources, SYS_RES_IRQ, 0);
	if (rle != NULL)
		r->irq = rle->start;
	r->base = r->len = 0;
	error = 0;
	if (r->reg != 0) {
		rle = resource_list_find(&dinfo->resources, SYS_RES_MEMORY,
		    r->reg);
		/*
		 * A BAR the firmware left unassigned (base 0: QEMU's virt has
		 * no firmware that assigns them) is assigned when a driver
		 * allocates it (pci_alloc_resource(), pci_reserve_map()). The
		 * driver is in another server: pcid allocates it for it, and
		 * keeps the reservation.
		 */
		if (rle == NULL) {
			int rid = r->reg;

			(void)bus_alloc_resource_any(dev, SYS_RES_MEMORY, &rid,
			    0);
			rle = resource_list_find(&dinfo->resources,
			    SYS_RES_MEMORY, r->reg);
		}
		if (rle == NULL)
			error = ENOENT;
		else {
			error = bus_translate_resource(dev, SYS_RES_MEMORY,
			    rle->start, &cpu);
			r->base = cpu;
			r->len = rle->count;
		}
	}
	bus_topo_unlock();
	return (error);
// ANCHOR_END: pci_route
}

int
bsdkern_cdev_ioctl_packed(struct bsdkern_open *o, u_long cmd, void *buf,
    size_t len, size_t *out)
{
	struct pci_conf_io *pc = buf;
	struct pci_match_conf *upat;
	struct pci_conf *umatch;
	char *p = buf;
	int error;

	if (cmd == PCIOCROUTE) {
		*out = len;
		return (len < sizeof(struct bsdkern_pci_route) ? EINVAL :
		    route(buf));
	}
	if (cmd != PCIOCGETCONF)
		return (-1);
	if (len < sizeof(*pc) ||
	    (size_t)pc->pat_buf_len + pc->match_buf_len > len - sizeof(*pc))
		return (EINVAL);
	upat = pc->patterns;
	umatch = pc->matches;
	pc->patterns = pc->num_patterns != 0 ?
	    (struct pci_match_conf *)(p + sizeof(*pc)) : NULL;
	pc->matches = (struct pci_conf *)(p + sizeof(*pc) + pc->pat_buf_len);
	error = bsdkern_cdev_ioctl(o, cmd, buf);
	/* The client's own pointers back, for its copy of the struct. */
	pc->patterns = upat;
	pc->matches = umatch;
	*out = len;
	return (error);
}

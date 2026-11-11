/*
 * FreeBSD's interrupt framework (INTRNG, kern/subr_intr.c), as much of it
 * as a server with interrupt controllers of its own needs: a PCIe host
 * bridge is one, for its devices' MSIs. The system's
 * interrupt controller is the kernel's, reached through the nexus
 * (InterruptAttachThread()); the device tree's specifiers map onto it in
 * fdt.c.
 *
 * MSI sources register, but are not handed out yet: allocating one fails,
 * and FreeBSD's drivers fall back to their legacy interrupt (INTx), which
 * the host bridge routes to a GIC line of its own. Forwarding a bridge's
 * MSI to the server whose driver owns the device is the PCI client
 * layer's work.
 */
#ifdef INTRNG
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/intr.h>
#include <machine/stdarg.h>

int
intr_isrc_register(struct intr_irqsrc *isrc, device_t dev, u_int flags,
    const char *fmt, ...)
{
	va_list ap;

	bzero(isrc, sizeof(*isrc));
	isrc->isrc_dev = dev;
	isrc->isrc_flags = flags;
	va_start(ap, fmt);
	vsnprintf(isrc->isrc_name, INTR_ISRC_NAMELEN, fmt, ap);
	va_end(ap);
	return (0);
}

/* An MSI no driver was given: nothing to run. */
int
intr_isrc_dispatch(struct intr_irqsrc *isrc, struct trapframe *tf)
{
	(void)isrc;
	(void)tf;
	return (EINVAL);
}

int
intr_msi_register(device_t dev, intptr_t xref)
{
	(void)dev;
	(void)xref;
	return (0);
}

int
intr_alloc_msi(device_t pci, device_t child, intptr_t xref, int count,
    int maxcount, int *irqs)
{
	return (ENXIO);
}

int
intr_release_msi(device_t pci, device_t child, intptr_t xref, int count,
    int *irqs)
{
	return (EINVAL);
}

int
intr_map_msi(device_t pci, device_t child, intptr_t xref, int irq,
    uint64_t *addr, uint32_t *data)
{
	return (EINVAL);
}

int
intr_alloc_msix(device_t pci, device_t child, intptr_t xref, int *irq)
{
	return (ENXIO);
}

int
intr_release_msix(device_t pci, device_t child, intptr_t xref, int irq)
{
	return (EINVAL);
}
#endif

/*
 * OsCraft's own request on /dev/pci, between pcid and the PCI client bus
 * of a server holding drivers (kern/pciclient.c): what a driver needs of
 * its device beyond configuration space, which FreeBSD's PCIOCGETBAR
 * doesn't say: a window's address as the CPU sees it (through the host
 * bridge's window, not the PCI bus address) and the interrupt the bridge
 * routes its INTx pin to (a system-page vector).
 */
#ifndef _BSDKERN_PCI_H_
#define	_BSDKERN_PCI_H_

#include <sys/ioccom.h>
#include <sys/pciio.h>

// ANCHOR: pci_route_request
struct bsdkern_pci_route {
	struct pcisel	sel;
	uint32_t	reg;	/* a BAR's register (PCIR_BAR(n)), or 0 */
	uint32_t	pad;
	uint64_t	base;	/* the BAR's CPU physical address */
	uint64_t	len;
	int32_t		irq;	/* INTx's vector, -1 for none */
	uint32_t	pad2;
};

#define	PCIOCROUTE	_IOWR('p', 100, struct bsdkern_pci_route)
// ANCHOR_END: pci_route_request

#endif

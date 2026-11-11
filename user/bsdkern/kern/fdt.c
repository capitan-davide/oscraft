/*
 * The board's device tree, for servers built with FDT (pcid): startup
 * leaves the firmware's tree in memory and records it in the system page
 * ("fdt" in asinfo). It is mapped read-only and handed to FreeBSD's FDT
 * layer as the kernel's machdep does (OF_install(), OF_init()), before
 * newbus configures: ofwbus then attaches under the nexus, simplebus
 * under it, and the board's devices under simplebus, from the tree.
 *
 * Interrupts: a device tree names them as its interrupt controller's
 * specifiers; FreeBSD's INTRNG maps those through the controller's driver.
 * Here the controller is the kernel's: the GIC's specifiers (type, number,
 * flags) become the system page's vector numbers, the GIC's own interrupt
 * IDs (SPI n: 32 + n, PPI n: 16 + n), which the nexus attaches to.
 */
#include "opt_platform.h"

#ifdef FDT
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/kernel.h>

#include <dev/ofw/openfirm.h>

bool	bsdkern_syspage_as(const char *, uint64_t *, uint64_t *);
void	*bsdkern_map_table(uint64_t, size_t);

static void
fdt_start(void *arg)
{
	uint64_t start, end, off;
	char *blob;

	(void)arg;
	if (!bsdkern_syspage_as("fdt", &start, &end))
		panic("fdt: no device tree in the system page");
	off = start & PAGE_MASK;
	blob = bsdkern_map_table(start - off, round_page(end + 1 - start + off));
	if (blob == NULL)
		panic("fdt: device tree not mapped");
	if (!OF_install(OFW_FDT, 0))
		panic("fdt: OF_install");
	if (OF_init(blob + off) != 0)
		panic("fdt: OF_init");
}
/* Before openfirm.c's xrefinfo_init() (SI_SUB_KMEM) reads the tree. */
SYSINIT(bsdkern_fdt, SI_SUB_VM, SI_ORDER_FIRST, fdt_start, NULL);

bool	bsdkern_fdt_present(void);

/* For the nexus, which adds ofwbus. */
bool
bsdkern_fdt_present(void)
{
	return (true);
}

/* GIC specifiers: <type number flags>, type 0 SPI, 1 PPI. */
#define	GIC_SPI		0
#define	GIC_PPI		1

u_int	intr_fdt_map_irq(phandle_t, pcell_t *, u_int);

// ANCHOR: fdt_map_irq
u_int
intr_fdt_map_irq(phandle_t node, pcell_t *cells, u_int ncells)
{
	(void)node;
	if (ncells < 2)
		return (cells[0]);
	switch (cells[0]) {
	case GIC_SPI:
		return (32 + cells[1]);
	case GIC_PPI:
		return (16 + cells[1]);
	default:
		return (cells[1]);
	}
// ANCHOR_END: fdt_map_irq
}
#endif

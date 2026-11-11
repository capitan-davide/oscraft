/*
 * usbd's own glue to FreeBSD's USB core: what its packet capture
 * (usb_pf.c, which needs the network stack's bpf and interfaces) would do.
 * usbd has no network stack: no capture, so the taps do nothing.
 */
#include <sys/param.h>
#include <sys/systm.h>

struct usb_bus;
struct usb_xfer;

void	usbpf_attach(struct usb_bus *);
void	usbpf_detach(struct usb_bus *);
void	usbpf_xfertap(struct usb_xfer *, int);

void
usbpf_attach(struct usb_bus *bus)
{
	(void)bus;
}

void
usbpf_detach(struct usb_bus *bus)
{
	(void)bus;
}

void
usbpf_xfertap(struct usb_xfer *xfer, int type)
{
	(void)xfer;
	(void)type;
}

/*
 * The network stack's, which FreeBSD's generic code mentions: m_apply() for a
 * class driver copying a packet into a transfer (usbd_m_copy_in()), which
 * happens in the server holding the driver and its mbufs (netd), not here.
 */
struct mbuf;
int	m_apply(struct mbuf *, int, int, int (*)(void *, void *, u_int), void *);

int
m_apply(struct mbuf *m, int off, int len, int (*f)(void *, void *, u_int), void *arg)
{
	(void)m; (void)off; (void)len; (void)f; (void)arg;
	panic("m_apply: no mbufs in usbd");
}

/*
 * usbd's "-o <sysctl>=<value>": an integer sysctl of FreeBSD's USB code
 * (hw.usb.debug, hw.usb.dwc_otg.debug, ...). 0, or an errno.
 */
#include <sys/proc.h>
#include <sys/sysctl.h>

int	usbd_sysctl_int(const char *, int);

int
usbd_sysctl_int(const char *name, int value)
{
	return (kernel_sysctlbyname(curthread, __DECONST(char *, name), NULL,
	    NULL, &value, sizeof(value), NULL, 0));
}

/*
 * A USB host controller in a server holding class drivers (netd: if_cdce),
 * over usbd, which owns the real ones (user/usbd): FreeBSD's USB core runs
 * here unchanged, and so do its class drivers, on this controller, whose
 * root hub has a port for each device usbd lends (bsdkern_usb.h), and
 * whose pipes hand each transfer to usbd: a thread per endpoint sends
 * each frame to /dev/usbd and completes the transfer with the answer, as a
 * controller's interrupt would. Like a USB/IP client, over messages.
 *
 * What stays here: the root hub (emulated, as every controller driver
 * does: roothub_exec), SET_ADDRESS (the device has its address in usbd),
 * and the transfers' bookkeeping. What usbd does: the requests and
 * transfers on the wire, SET_INTERFACE through its own core.
 *
 * Built into a module of the server (usb.ko, with FreeBSD's USB core).
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/condvar.h>
#include <sys/kernel.h>
#include <sys/kthread.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/module.h>
#include <sys/mutex.h>
#include <sys/proc.h>
#include <sys/queue.h>
#include <sys/sx.h>

#include <dev/usb/usb.h>
#include <dev/usb/usbdi.h>
#include <dev/usb/usb_core.h>
#include <dev/usb/usb_busdma.h>
#include <dev/usb/usb_process.h>
#include <dev/usb/usb_transfer.h>
#include <dev/usb/usb_device.h>
#include <dev/usb/usb_hub.h>
#include <dev/usb/usb_util.h>
#include <dev/usb/usb_controller.h>
#include <dev/usb/usb_bus.h>

#include "bsdkern_usb.h"

/* libc's, in this process. */
int	open(const char *, int, ...);
int	ioctl(int, unsigned long, ...);
#define	O_RDWR	0x0002

#define	USBCL_PORTS	4
#define	USBCL_DEVICES	(USBCL_PORTS + 2)
#define	USBCL_WORKERS	32

static MALLOC_DEFINE(M_USBCL, "usbclient", "the USB client controller");

struct usbcl_softc;

/* A thread for one endpoint of one device: its transfers, in order. */
struct usbcl_worker {
	struct usbcl_softc	*sc;
	struct usb_device	*udev;
	uint8_t			 ep;	/* address with UE_DIR_IN; 0: control */
	TAILQ_HEAD(, usb_xfer)	 queue;	/* through xfer->wait_entry */
	struct usb_xfer		*busy;	/* sent, its answer awaited */
	struct cv		 cv;
};

struct usbcl_port {
	struct bsdkern_usb_dev	dev;
	uint16_t		status;
	uint16_t		change;
};

struct usbcl_softc {
	struct usb_bus		sc_bus;		/* first: the core's */
	struct usb_device	*sc_devices[USBCL_DEVICES];
	device_t		sc_dev;
	int			sc_fd;
	int			sc_nports;
	struct usbcl_port	sc_port[USBCL_PORTS];
	struct usbcl_worker	sc_worker[USBCL_WORKERS];
	int			sc_nworkers;
	uint8_t			sc_hub_idata[1];
	union {
		struct usb_port_status ps;
		struct usb_status st;
		struct usb_hub_descriptor_min hd;
		uint8_t wValue[2];
	} sc_hub_temp;
};

#define	BUS2SC(bus)	__containerof(bus, struct usbcl_softc, sc_bus)

/* The root hub's descriptors: a USB 2.0 hub of sc_nports ports. */
static const struct usb_device_descriptor usbcl_devd = {
	.bLength = sizeof(struct usb_device_descriptor),
	.bDescriptorType = UDESC_DEVICE,
	.bcdUSB = {0x00, 0x02},
	.bDeviceClass = UDCLASS_HUB,
	.bDeviceSubClass = UDSUBCLASS_HUB,
	.bDeviceProtocol = UDPROTO_HSHUBSTT,
	.bMaxPacketSize = 64,
	.bcdDevice = {0x00, 0x01},
	.iManufacturer = 1,
	.iProduct = 2,
	.bNumConfigurations = 1,
};

struct usbcl_config_desc {
	struct usb_config_descriptor confd;
	struct usb_interface_descriptor ifcd;
	struct usb_endpoint_descriptor endpd;
} __packed;

static const struct usbcl_config_desc usbcl_confd = {
	.confd = {
		.bLength = sizeof(struct usb_config_descriptor),
		.bDescriptorType = UDESC_CONFIG,
		.wTotalLength[0] = sizeof(usbcl_confd),
		.bNumInterface = 1,
		.bConfigurationValue = 1,
		.bmAttributes = UC_SELF_POWERED,
	},
	.ifcd = {
		.bLength = sizeof(struct usb_interface_descriptor),
		.bDescriptorType = UDESC_INTERFACE,
		.bNumEndpoints = 1,
		.bInterfaceClass = UICLASS_HUB,
		.bInterfaceSubClass = UISUBCLASS_HUB,
		.bInterfaceProtocol = 0,
	},
	.endpd = {
		.bLength = sizeof(struct usb_endpoint_descriptor),
		.bDescriptorType = UDESC_ENDPOINT,
		.bEndpointAddress = UE_DIR_IN | 1,
		.bmAttributes = UE_INTERRUPT,
		.wMaxPacketSize[0] = 8,
		.bInterval = 255,
	},
};

#define	STRING_VENDOR \
  "O\0s\0C\0r\0a\0f\0t"
#define	STRING_PRODUCT \
  "u\0s\0b\0d\0 \0c\0l\0i\0e\0n\0t"
USB_MAKE_STRING_DESC(STRING_VENDOR, usbcl_vendor);
USB_MAKE_STRING_DESC(STRING_PRODUCT, usbcl_product);

static usb_error_t
usbcl_roothub_exec(struct usb_device *udev, struct usb_device_request *req,
    const void **pptr, uint16_t *plength)
{
	struct usbcl_softc *sc = BUS2SC(udev->bus);
	const void *ptr = NULL;
	uint16_t len = 0, value, index;
	struct usbcl_port *p;

	USB_BUS_LOCK_ASSERT(&sc->sc_bus, MA_OWNED);
	value = UGETW(req->wValue);
	index = UGETW(req->wIndex);

#define	C(x, y)	((x) | ((y) << 8))
	switch (C(req->bRequest, req->bmRequestType)) {
	case C(UR_CLEAR_FEATURE, UT_WRITE_DEVICE):
	case C(UR_CLEAR_FEATURE, UT_WRITE_INTERFACE):
	case C(UR_CLEAR_FEATURE, UT_WRITE_ENDPOINT):
	case C(UR_SET_ADDRESS, UT_WRITE_DEVICE):
	case C(UR_SET_CONFIG, UT_WRITE_DEVICE):
	case C(UR_SET_FEATURE, UT_WRITE_DEVICE):
	case C(UR_SET_INTERFACE, UT_WRITE_INTERFACE):
	case C(UR_CLEAR_FEATURE, UT_WRITE_CLASS_DEVICE):
	case C(UR_SET_DESCRIPTOR, UT_WRITE_CLASS_DEVICE):
	case C(UR_SET_FEATURE, UT_WRITE_CLASS_DEVICE):
		break;
	case C(UR_GET_CONFIG, UT_READ_DEVICE):
		sc->sc_hub_temp.wValue[0] = 1;
		ptr = sc->sc_hub_temp.wValue;
		len = 1;
		break;
	case C(UR_GET_DESCRIPTOR, UT_READ_DEVICE):
		switch (value >> 8) {
		case UDESC_DEVICE:
			ptr = &usbcl_devd;
			len = sizeof(usbcl_devd);
			break;
		case UDESC_CONFIG:
			ptr = &usbcl_confd;
			len = sizeof(usbcl_confd);
			break;
		case UDESC_STRING:
			switch (value & 0xff) {
			case 0:
				ptr = &usb_string_lang_en;
				len = sizeof(usb_string_lang_en);
				break;
			case 1:
				ptr = &usbcl_vendor;
				len = sizeof(usbcl_vendor);
				break;
			case 2:
				ptr = &usbcl_product;
				len = sizeof(usbcl_product);
				break;
			default:
				return (USB_ERR_STALLED);
			}
			break;
		default:
			return (USB_ERR_STALLED);
		}
		break;
	case C(UR_GET_INTERFACE, UT_READ_INTERFACE):
		sc->sc_hub_temp.wValue[0] = 0;
		ptr = sc->sc_hub_temp.wValue;
		len = 1;
		break;
	case C(UR_GET_STATUS, UT_READ_DEVICE):
		USETW(sc->sc_hub_temp.st.wStatus, UDS_SELF_POWERED);
		ptr = &sc->sc_hub_temp.st;
		len = 2;
		break;
	case C(UR_GET_STATUS, UT_READ_INTERFACE):
	case C(UR_GET_STATUS, UT_READ_ENDPOINT):
		USETW(sc->sc_hub_temp.st.wStatus, 0);
		ptr = &sc->sc_hub_temp.st;
		len = 2;
		break;
	case C(UR_GET_DESCRIPTOR, UT_READ_CLASS_DEVICE):
		if ((value & 0xff) != 0)
			return (USB_ERR_STALLED);
		bzero(&sc->sc_hub_temp.hd, sizeof(sc->sc_hub_temp.hd));
		sc->sc_hub_temp.hd.bDescLength = sizeof(sc->sc_hub_temp.hd);
		sc->sc_hub_temp.hd.bDescriptorType = UDESC_HUB;
		sc->sc_hub_temp.hd.bNbrPorts = sc->sc_nports;
		USETW(sc->sc_hub_temp.hd.wHubCharacteristics, UHD_PWR_NO_SWITCH);
		sc->sc_hub_temp.hd.bPwrOn2PwrGood = 1;
		ptr = &sc->sc_hub_temp.hd;
		len = sizeof(sc->sc_hub_temp.hd);
		break;
	case C(UR_GET_STATUS, UT_READ_CLASS_DEVICE):
		bzero(&sc->sc_hub_temp.ps, 4);
		ptr = &sc->sc_hub_temp.ps;
		len = 4;
		break;
	case C(UR_GET_STATUS, UT_READ_CLASS_OTHER):
		if (index < 1 || index > sc->sc_nports)
			return (USB_ERR_IOERROR);
		p = &sc->sc_port[index - 1];
		USETW(sc->sc_hub_temp.ps.wPortStatus, p->status);
		USETW(sc->sc_hub_temp.ps.wPortChange, p->change);
		ptr = &sc->sc_hub_temp.ps;
		len = sizeof(sc->sc_hub_temp.ps);
		break;
	case C(UR_SET_FEATURE, UT_WRITE_CLASS_OTHER):
		if (index < 1 || index > sc->sc_nports)
			return (USB_ERR_IOERROR);
		p = &sc->sc_port[index - 1];
		switch (value) {
		case UHF_PORT_RESET:
			/* The device was reset in usbd: enabled at once. */
			p->status |= UPS_PORT_ENABLED;
			p->change |= UPS_C_PORT_RESET;
			break;
		case UHF_PORT_POWER:
			p->status |= UPS_PORT_POWER;
			break;
		default:
			break;
		}
		break;
	case C(UR_CLEAR_FEATURE, UT_WRITE_CLASS_OTHER):
		if (index < 1 || index > sc->sc_nports)
			return (USB_ERR_IOERROR);
		p = &sc->sc_port[index - 1];
		switch (value) {
		case UHF_C_PORT_CONNECTION:
			p->change &= ~UPS_C_CONNECT_STATUS;
			break;
		case UHF_C_PORT_RESET:
			p->change &= ~UPS_C_PORT_RESET;
			break;
		case UHF_C_PORT_ENABLE:
			p->change &= ~UPS_C_PORT_ENABLED;
			break;
		case UHF_PORT_ENABLE:
			p->status &= ~UPS_PORT_ENABLED;
			break;
		default:
			break;
		}
		break;
	default:
		return (USB_ERR_STALLED);
	}
#undef C
	*pptr = ptr;
	*plength = len;
	return (USB_ERR_NORMAL_COMPLETION);
}

/* The device behind a root port: usbd's ID. */
static uint16_t
usbcl_id(struct usbcl_softc *sc, struct usb_device *udev)
{
	struct usb_device *d = udev;

	/* Up to the device on a root port. */
	while (d->parent_hub != NULL && d->parent_hub->parent_hub != NULL)
		d = d->parent_hub;
	if (d->port_no < 1 || d->port_no > sc->sc_nports)
		return (0);
	return (sc->sc_port[d->port_no - 1].dev.id);
}

/* A frame or a request to usbd, then the transfer completed. */
static void
usbcl_run(struct usbcl_worker *w, struct usb_xfer *xfer)
{
	struct usbcl_softc *sc = w->sc;
	struct bsdkern_usb_ctrl *c = NULL;
	struct bsdkern_usb_xfer *x = NULL;
	usb_error_t err = 0;
	uint16_t id;
	uint32_t n, len;

	id = usbcl_id(sc, xfer->xroot->udev);
	if (w->ep == 0) {
		c = malloc(sizeof(*c), M_USBCL, M_WAITOK | M_ZERO);
		c->id = id;
		c->timeout = xfer->timeout;
		usbd_copy_out(xfer->frbuffers, 0, c->req, sizeof(c->req));
		len = xfer->nframes > 1 ? xfer->frlengths[1] : 0;
		if (len > sizeof(c->data)) {
			err = USB_ERR_INVAL;
		} else {
			if (len != 0 && (c->req[0] & UT_READ) == 0)
				usbd_copy_out(xfer->frbuffers + 1, 0, c->data,
				    len);
			if (ioctl(sc->sc_fd, USBDIOCCTRL, c) != 0)
				err = USB_ERR_IOERROR;
			else
				err = c->error;
		}
	} else {
		x = malloc(sizeof(*x), M_USBCL, M_WAITOK | M_ZERO);
		x->id = id;
		x->ep = w->ep;
		x->type = xfer->endpoint->edesc->bmAttributes & UE_XFERTYPE;
		x->timeout = xfer->timeout;
	}
	USB_BUS_LOCK(&sc->sc_bus);
	/* Closed meanwhile (stopped, timed out): the answer is dropped. */
	if (w->busy != xfer) {
		USB_BUS_UNLOCK(&sc->sc_bus);
		goto out;
	}
	USB_BUS_UNLOCK(&sc->sc_bus);
	if (w->ep == 0) {
		if (err == 0 && (c->req[0] & UT_READ) != 0 && c->actlen != 0)
			usbd_copy_in(xfer->frbuffers + 1, 0, c->data,
			    c->actlen);
		if (xfer->nframes > 1)
			xfer->frlengths[1] = c->actlen;
		xfer->aframes = xfer->nframes;
	} else {
		/* Frame by frame: each its own transfer on the wire. */
		for (n = 0; n < xfer->nframes && err == 0; n++) {
			len = xfer->frlengths[n];
			if (len > sizeof(x->data)) {
				err = USB_ERR_INVAL;
				break;
			}
			x->len = len;
			if ((w->ep & UE_DIR_IN) == 0)
				usbd_copy_out(xfer->frbuffers + n, 0, x->data,
				    len);
			if (ioctl(sc->sc_fd, USBDIOCXFER, x) != 0)
				err = USB_ERR_IOERROR;
			else
				err = x->error;
			if (err != 0)
				break;
			if ((w->ep & UE_DIR_IN) != 0)
				usbd_copy_in(xfer->frbuffers + n, 0, x->data,
				    x->actlen);
			xfer->frlengths[n] = x->actlen;
			if (x->actlen < len) {
				/* A short frame ends the transfer. */
				if (!xfer->flags_int.short_xfer_ok)
					err = USB_ERR_SHORT_XFER;
				n++;
				break;
			}
		}
		xfer->aframes = n;
	}
	USB_BUS_LOCK(&sc->sc_bus);
	if (w->busy == xfer) {
		w->busy = NULL;
		usbd_transfer_done(xfer, err);
	}
	USB_BUS_UNLOCK(&sc->sc_bus);
out:
	free(c, M_USBCL);
	free(x, M_USBCL);
}

static void
usbcl_worker_loop(void *arg)
{
	struct usbcl_worker *w = arg;
	struct usbcl_softc *sc = w->sc;
	struct usb_xfer *xfer;

	USB_BUS_LOCK(&sc->sc_bus);
	for (;;) {
		while ((xfer = TAILQ_FIRST(&w->queue)) == NULL)
			cv_wait(&w->cv, &sc->sc_bus.bus_mtx);
		TAILQ_REMOVE(&w->queue, xfer, wait_entry);
		xfer->td_transfer_cache = NULL;
		w->busy = xfer;
		USB_BUS_UNLOCK(&sc->sc_bus);
		usbcl_run(w, xfer);
		USB_BUS_LOCK(&sc->sc_bus);
	}
}

static struct usbcl_worker *
usbcl_worker(struct usbcl_softc *sc, struct usb_xfer *xfer)
{
	struct usb_device *udev = xfer->xroot->udev;
	struct usbcl_worker *w;
	uint8_t ep = xfer->endpoint->edesc->bEndpointAddress;
	int i;

	if ((xfer->endpoint->edesc->bmAttributes & UE_XFERTYPE) ==
	    UE_CONTROL)
		ep = 0;
	USB_BUS_LOCK_ASSERT(&sc->sc_bus, MA_OWNED);
	for (i = 0; i < sc->sc_nworkers; i++) {
		w = &sc->sc_worker[i];
		if (w->udev == udev && w->ep == ep)
			return (w);
	}
	if (sc->sc_nworkers == USBCL_WORKERS)
		return (NULL);
	w = &sc->sc_worker[sc->sc_nworkers++];
	w->sc = sc;
	w->udev = udev;
	w->ep = ep;
	TAILQ_INIT(&w->queue);
	cv_init(&w->cv, "usbcl");
	kthread_add(usbcl_worker_loop, w, NULL, NULL, 0, 0, "usbcl %d.%x",
	    udev->address, ep);
	return (w);
}

static void
usbcl_pipe_open(struct usb_xfer *xfer)
{
}

/* The transfer is no longer the worker's, sent or queued. */
static void
usbcl_forget(struct usb_xfer *xfer)
{
	struct usbcl_softc *sc = BUS2SC(xfer->xroot->bus);
	struct usbcl_worker *w;
	int i;

	USB_BUS_LOCK_ASSERT(&sc->sc_bus, MA_OWNED);
	for (i = 0; i < sc->sc_nworkers; i++) {
		w = &sc->sc_worker[i];
		if (w->busy == xfer)
			w->busy = NULL;
	}
	/* Queued, not sent: the controller's own field says where. */
	if ((w = xfer->td_transfer_cache) != NULL) {
		TAILQ_REMOVE(&w->queue, xfer, wait_entry);
		xfer->td_transfer_cache = NULL;
	}
}

static void
usbcl_pipe_close(struct usb_xfer *xfer)
{
	usbcl_forget(xfer);
	usbd_transfer_done(xfer, USB_ERR_CANCELLED);
}

static void
usbcl_pipe_timeout(void *arg)
{
	struct usb_xfer *xfer = arg;

	usbcl_forget(xfer);
	usbd_transfer_done(xfer, USB_ERR_TIMEOUT);
}

static void
usbcl_pipe_enter(struct usb_xfer *xfer)
{
}

// ANCHOR: usbcl_pipe_start
static void
usbcl_pipe_start(struct usb_xfer *xfer)
{
	struct usbcl_softc *sc = BUS2SC(xfer->xroot->bus);
	struct usbcl_worker *w;

	w = usbcl_worker(sc, xfer);
	if (w == NULL) {
		usbd_transfer_done(xfer, USB_ERR_NOMEM);
		return;
	}
	if (xfer->timeout != 0)
		usbd_transfer_timeout_ms(xfer, &usbcl_pipe_timeout,
		    xfer->timeout);
	xfer->td_transfer_cache = w;
	TAILQ_INSERT_TAIL(&w->queue, xfer, wait_entry);
	cv_signal(&w->cv);
// ANCHOR_END: usbcl_pipe_start
}

static const struct usb_pipe_methods usbcl_pipe_methods = {
	.open = usbcl_pipe_open,
	.close = usbcl_pipe_close,
	.enter = usbcl_pipe_enter,
	.start = usbcl_pipe_start,
};

static void
usbcl_endpoint_init(struct usb_device *udev,
    struct usb_endpoint_descriptor *edesc, struct usb_endpoint *ep)
{
	/* Isochronous transfers are not lent. */
	if ((edesc->bmAttributes & UE_XFERTYPE) == UE_ISOCHRONOUS)
		return;
	/* The root hub's requests are roothub_exec()'s. */
	if (udev->parent_hub == NULL)
		return;
	ep->methods = &usbcl_pipe_methods;
}

static void
usbcl_xfer_setup(struct usb_setup_params *parm)
{
	parm->hc_max_packet_size = 0x400;
	parm->hc_max_packet_count = 3;
	parm->hc_max_frame_size = BSDKERN_USB_DATA;
	usbd_transfer_setup_sub(parm);
}

static void
usbcl_xfer_unsetup(struct usb_xfer *xfer)
{
}

static void
usbcl_get_dma_delay(struct usb_device *udev, uint32_t *pus)
{
	*pus = 0;
}

static void
usbcl_device_nop(struct usb_device *udev)
{
}

static void
usbcl_set_hw_power(struct usb_bus *bus)
{
}

/* The device's address is usbd's: here only the core's record of it. */
static usb_error_t
usbcl_set_address(struct usb_device *udev, struct mtx *mtx, uint16_t addr)
{
	return (0);
}

static const struct usb_bus_methods usbcl_bus_methods = {
	.endpoint_init = usbcl_endpoint_init,
	.xfer_setup = usbcl_xfer_setup,
	.xfer_unsetup = usbcl_xfer_unsetup,
	.get_dma_delay = usbcl_get_dma_delay,
	.device_suspend = usbcl_device_nop,
	.device_resume = usbcl_device_nop,
	.set_hw_power = usbcl_set_hw_power,
	.roothub_exec = usbcl_roothub_exec,
	.set_address = usbcl_set_address,
};

static int
usbcl_probe(device_t dev)
{
	device_set_desc(dev, "USB host controller, through usbd");
	return (BUS_PROBE_GENERIC);
}

/*
 * The ports follow usbd's devices: a device not on a port takes a free
 * one (connected, a connection change), a port whose device is gone is
 * disconnected (a change too). The bus lock held; whether anything
 * changed, for the hub to explore.
 */
static bool
usbcl_sync(struct usbcl_softc *sc, const struct bsdkern_usb_list *l)
{
	struct usbcl_port *p;
	bool changed = false, there;
	uint32_t i;
	int n;

	USB_BUS_LOCK_ASSERT(&sc->sc_bus, MA_OWNED);
	for (n = 0; n < sc->sc_nports; n++) {
		p = &sc->sc_port[n];
		if (p->dev.id == 0)
			continue;
		there = false;
		for (i = 0; i < l->n; i++)
			there |= l->dev[i].id == p->dev.id;
		if (!there) {
			p->dev.id = 0;
			p->status &= ~(UPS_CURRENT_CONNECT_STATUS |
			    UPS_PORT_ENABLED | UPS_LOW_SPEED | UPS_HIGH_SPEED);
			p->change |= UPS_C_CONNECT_STATUS;
			changed = true;
		}
	}
	/* The devices usbd lends: not its hubs. */
	for (i = 0; i < l->n; i++) {
		if (l->dev[i].hub)
			continue;
		there = false;
		for (n = 0; n < sc->sc_nports; n++)
			there |= sc->sc_port[n].dev.id == l->dev[i].id;
		if (there)
			continue;
		for (n = 0; n < sc->sc_nports && sc->sc_port[n].dev.id != 0; n++)
			;
		if (n == sc->sc_nports)
			break;
		p = &sc->sc_port[n];
		p->dev = l->dev[i];
		p->status = UPS_CURRENT_CONNECT_STATUS | UPS_PORT_POWER;
		if (p->dev.speed == USB_SPEED_LOW)
			p->status |= UPS_LOW_SPEED;
		else if (p->dev.speed == USB_SPEED_HIGH)
			p->status |= UPS_HIGH_SPEED;
		p->change |= UPS_C_CONNECT_STATUS;
		changed = true;
		device_printf(sc->sc_dev, "port %d: usbd's device %d.%d, "
		    "%04x:%04x\n", n + 1, p->dev.id >> 8, p->dev.id & 0xff,
		    UGETW(((struct usb_device_descriptor *)p->dev.desc)->idVendor),
		    UGETW(((struct usb_device_descriptor *)p->dev.desc)->idProduct));
	}
	return (changed);
}

/*
 * Hot-plug: waits in usbd for its devices to change (USBDIOCWAIT), then
 * lists them again; the root hub reports the ports' changes to the core's
 * hub driver (uhub_root_intr()), which explores them: a new device is
 * enumerated, a gone one detached, here as on any controller.
 */
static void
usbcl_watch(void *arg)
{
	struct usbcl_softc *sc = arg;
	struct bsdkern_usb_list *l;
	uint32_t gen = ~0u;	/* the first wait answers at once */

	l = malloc(sizeof(*l), M_USBCL, M_WAITOK | M_ZERO);
	for (;;) {
		if (ioctl(sc->sc_fd, USBDIOCWAIT, &gen) != 0 ||
		    ioctl(sc->sc_fd, USBDIOCLIST, l) != 0) {
			pause("usbclw", hz);
			continue;
		}
		USB_BUS_LOCK(&sc->sc_bus);
		if (usbcl_sync(sc, l))
			uhub_root_intr(&sc->sc_bus, NULL, 0);
		USB_BUS_UNLOCK(&sc->sc_bus);
	}
}

static int
usbcl_attach(device_t dev)
{
	struct usbcl_softc *sc = device_get_softc(dev);
	struct bsdkern_usb_list *l;
	int error;

	sc->sc_dev = dev;
	sc->sc_fd = open("/dev/usbd", O_RDWR);
	if (sc->sc_fd < 0) {
		device_printf(dev, "no /dev/usbd: is usbd running?\n");
		return (ENXIO);
	}
	/* Fixed: the hub's port count is read once, at its attach. */
	sc->sc_nports = USBCL_PORTS;
	l = malloc(sizeof(*l), M_USBCL, M_WAITOK | M_ZERO);
	if (ioctl(sc->sc_fd, USBDIOCLIST, l) != 0) {
		free(l, M_USBCL);
		return (ENXIO);
	}
	sc->sc_bus.parent = dev;
	sc->sc_bus.devices = sc->sc_devices;
	sc->sc_bus.devices_max = USBCL_DEVICES;
	sc->sc_bus.dma_bits = 32;
	sc->sc_bus.usbrev = USB_REV_2_0;
	sc->sc_bus.methods = &usbcl_bus_methods;
	if (usb_bus_mem_alloc_all(&sc->sc_bus, USB_GET_DMA_TAG(dev), NULL)) {
		free(l, M_USBCL);
		return (ENOMEM);
	}
	USB_BUS_LOCK(&sc->sc_bus);
	(void)usbcl_sync(sc, l);
	USB_BUS_UNLOCK(&sc->sc_bus);
	free(l, M_USBCL);
	sc->sc_bus.bdev = device_add_child(dev, "usbus", DEVICE_UNIT_ANY);
	if (sc->sc_bus.bdev == NULL)
		return (ENXIO);
	device_set_ivars(sc->sc_bus.bdev, &sc->sc_bus);
	error = device_probe_and_attach(sc->sc_bus.bdev);
	if (error == 0)
		kthread_add(usbcl_watch, sc, NULL, NULL, 0, 0, "usbcl watch");
	return (error);
}

static device_method_t usbcl_methods[] = {
	DEVMETHOD(device_probe,		usbcl_probe),
	DEVMETHOD(device_attach,	usbcl_attach),
	DEVMETHOD(device_detach,	bus_generic_detach),
	DEVMETHOD(device_shutdown,	bus_generic_shutdown),
	DEVMETHOD_END
};

static driver_t usbcl_driver = {
	.name = "usbcl",
	.methods = usbcl_methods,
	.size = sizeof(struct usbcl_softc),
};

DRIVER_MODULE(usbcl, nexus, usbcl_driver, 0, 0);
MODULE_DEPEND(usbcl, usb, 1, 1, 1);

int	bsdkern_nexus_add(const char *, uint64_t, uint64_t, int);

/*
 * The core's bus driver (usbus, usb_controller.c) attaches under a list of
 * controllers' names; the same driver is registered under this one, from
 * where it is under dwcotg's. Then the controller comes up.
 */
static void
usbcl_start(void *arg)
{
	driver_t **drivers;
	devclass_t dc;
	int i, n, error;

	(void)arg;
	bus_topo_lock();
	dc = devclass_find("dwcotg");
	if (dc != NULL && devclass_get_drivers(dc, &drivers, &n) == 0) {
		for (i = 0; i < n; i++)
			if (strcmp(drivers[i]->name, "usbus") == 0)
				devclass_add_driver(
				    devclass_create("usbcl"),
				    drivers[i], BUS_PASS_DEFAULT, NULL);
		free(drivers, M_TEMP);
	}
	bus_topo_unlock();
	error = bsdkern_nexus_add("usbcl", 0, 0, -1);
	if (error != 0)
		printf("usbcl: not attached: %d\n", error);
}
SYSINIT(usbcl, SI_SUB_DRIVERS, SI_ORDER_ANY, usbcl_start, NULL);

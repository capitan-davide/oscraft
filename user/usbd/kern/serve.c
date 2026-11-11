/*
 * usbd's service to the servers holding class drivers (bsdkern_usb.h):
 * /dev/usbd, a cdev bsdkern's devfs serves, whose ioctls run requests and
 * transfers on usbd's devices through FreeBSD's USB core, for the client's
 * virtual host controller.
 */
#include <sys/param.h>
#include <sys/systm.h>
#include <sys/bus.h>
#include <sys/conf.h>
#include <sys/condvar.h>
#include <sys/kernel.h>
#include <sys/lock.h>
#include <sys/malloc.h>
#include <sys/module.h>
#include <sys/mutex.h>
#include <sys/sx.h>

#include <dev/usb/usb.h>
#include <dev/usb/usbdi.h>
#include <dev/usb/usbdi_util.h>
#include <dev/usb/usb_core.h>
#include <dev/usb/usb_busdma.h>
#include <dev/usb/usb_process.h>
#include <dev/usb/usb_device.h>
#include <dev/usb/usb_controller.h>
#include <dev/usb/usb_bus.h>
#include <dev/usb/usb_request.h>

#include "bsdkern_usb.h"

static void	pipes_drop(uint16_t, struct usb_device *);

static struct usb_device *
find(uint16_t id)
{
	devclass_t dc = devclass_find("usbus");
	struct usb_bus *bus;
	device_t d;
	u_int addr = id & 0xff;

	if (dc == NULL || (d = devclass_get_device(dc, id >> 8)) == NULL ||
	    (bus = device_get_ivars(d)) == NULL || addr >= bus->devices_max)
		return (NULL);
	return (bus->devices[addr]);
}

static int
list(struct bsdkern_usb_list *l)
{
	devclass_t dc = devclass_find("usbus");
	struct usb_device *udev;
	struct usb_bus *bus;
	device_t d;
	int unit, addr;

	bzero(l, sizeof(*l));
	if (dc == NULL)
		return (0);
	for (unit = 0; unit < devclass_get_maxunit(dc); unit++) {
		if ((d = devclass_get_device(dc, unit)) == NULL ||
		    (bus = device_get_ivars(d)) == NULL)
			continue;
		for (addr = 1; addr < bus->devices_max; addr++) {
			if ((udev = bus->devices[addr]) == NULL ||
			    l->n == BSDKERN_USB_DEVS)
				continue;
			l->dev[l->n].id = unit << 8 | addr;
			l->dev[l->n].speed = udev->speed;
			l->dev[l->n].hub = udev->hub != NULL;
			memcpy(l->dev[l->n].desc, &udev->ddesc, 18);
			l->n++;
		}
	}
	return (0);
}

static int
ctrl(struct bsdkern_usb_ctrl *c)
{
	struct usb_device_request req;
	struct usb_device *udev;
	uint16_t len, actlen = 0;

	if ((udev = find(c->id)) == NULL)
		return (ENXIO);
	memcpy(&req, c->req, sizeof(req));
	len = UGETW(req.wLength);
	if (len > sizeof(c->data))
		return (EINVAL);
// ANCHOR: usbd_ctrl_rules
	/* SET_INTERFACE: usbd's endpoints follow the alternate setting. */
	if (req.bmRequestType == UT_WRITE_INTERFACE &&
	    req.bRequest == UR_SET_INTERFACE) {
		pipes_drop(0, udev);
		c->error = usbd_set_alt_interface_index(udev,
		    UGETW(req.wIndex) & 0xff, UGETW(req.wValue) & 0xff);
		c->actlen = 0;
		return (0);
	}
	/* SET_CONFIGURATION to the one usbd set: nothing to do. */
	if (req.bmRequestType == UT_WRITE_DEVICE &&
	    req.bRequest == UR_SET_CONFIG && udev->cdesc != NULL &&
	    UGETW(req.wValue) == udev->cdesc->bConfigurationValue) {
		c->error = 0;
		c->actlen = 0;
		return (0);
	}
// ANCHOR_END: usbd_ctrl_rules
	c->error = usbd_do_request_flags(udev, NULL, &req, c->data,
	    USB_SHORT_XFER_OK, &actlen, c->timeout != 0 ? c->timeout : 5000);
	c->actlen = actlen;
	return (0);
}

/*
 * An endpoint's transfer, set up at its first frame and kept: a pipe. Its
 * frames go one at a time (the client has a thread per endpoint, and the
 * lock keeps two apart). Pipes go when the device does (its DETACH, from
 * usb_free_device() before the device is freed) and when its interface's
 * alternate setting changes (SET_INTERFACE): the endpoints are new then.
 */
struct upipe {
	LIST_ENTRY(upipe)	link;
	struct usb_device	*udev;
	uint16_t		id;
	uint8_t			ep;
	uint8_t			type;
	struct usb_xfer		*xf[1];
	struct sx		lock;	/* one frame at a time */
	struct mtx		mtx;	/* the transfer's */
	struct cv		cv;
	struct bsdkern_usb_xfer	*x;	/* the frame in flight */
	bool			done;
};

static LIST_HEAD(, upipe) pipes = LIST_HEAD_INITIALIZER(pipes);
static struct sx pipes_lock;
SX_SYSINIT(usbd_pipes, &pipes_lock, "usbd pipes");

/* The devices' generation: one more at each attach and detach. */
static struct mtx gen_mtx;
MTX_SYSINIT(usbd_gen, &gen_mtx, "usbd generation", MTX_DEF);
static uint32_t generation;

static void
frame_cb(struct usb_xfer *xfer, usb_error_t error)
{
	struct upipe *p = usbd_xfer_softc(xfer);
	int actlen;

	switch (USB_GET_STATE(xfer)) {
	case USB_ST_SETUP:
		if (p->x == NULL || p->done)
			return;
		usbd_xfer_set_frame_data(xfer, 0, p->x->data, p->x->len);
		usbd_xfer_set_frames(xfer, 1);
		usbd_xfer_set_timeout(xfer, p->x->timeout);
		usbd_transfer_submit(xfer);
		return;
	case USB_ST_TRANSFERRED:
		usbd_xfer_status(xfer, &actlen, NULL, NULL, NULL);
		p->x->actlen = actlen;
		p->x->error = 0;
		break;
	default:
		p->x->actlen = 0;
		p->x->error = error;
		break;
	}
	p->done = true;
	cv_signal(&p->cv);
}

static void
pipe_free(struct upipe *p)
{
	sx_xlock(&p->lock);
	usbd_transfer_unsetup(p->xf, 1);
	sx_xunlock(&p->lock);
	sx_destroy(&p->lock);
	cv_destroy(&p->cv);
	mtx_destroy(&p->mtx);
	free(p, M_DEVBUF);
}

/* The pipes of a device going (id) or changing interface (udev). */
static void
pipes_drop(uint16_t id, struct usb_device *udev)
{
	struct upipe *p, *next;
	LIST_HEAD(, upipe) gone = LIST_HEAD_INITIALIZER(gone);

	sx_xlock(&pipes_lock);
	LIST_FOREACH_SAFE(p, &pipes, link, next)
		if (p->id == id || (udev != NULL && p->udev == udev)) {
			LIST_REMOVE(p, link);
			LIST_INSERT_HEAD(&gone, p, link);
		}
	sx_xunlock(&pipes_lock);
	while ((p = LIST_FIRST(&gone)) != NULL) {
		LIST_REMOVE(p, link);
		pipe_free(p);
	}
}

static struct upipe *
pipe_get(struct usb_device *udev, struct bsdkern_usb_xfer *x)
{
	struct usb_config cfg;
	struct upipe *p;
	uint8_t i;
	int error;

	sx_xlock(&pipes_lock);
	LIST_FOREACH(p, &pipes, link)
		if (p->udev == udev && p->ep == x->ep && p->type == x->type)
			break;
	if (p != NULL) {
		sx_xunlock(&pipes_lock);
		return (p);
	}
	p = malloc(sizeof(*p), M_DEVBUF, M_WAITOK | M_ZERO);
	p->udev = udev;
	p->id = x->id;
	p->ep = x->ep;
	p->type = x->type;
	sx_init(&p->lock, "usbd pipe");
	mtx_init(&p->mtx, "usbd pipe", NULL, MTX_DEF);
	cv_init(&p->cv, "usbd pipe");
	bzero(&cfg, sizeof(cfg));
	cfg.type = x->type;
	cfg.endpoint = x->ep & UE_ADDR;
	cfg.direction = (x->ep & UE_DIR_IN) ? UE_DIR_IN : UE_DIR_OUT;
	cfg.bufsize = sizeof(x->data);
	cfg.frames = 1;
	cfg.flags.short_xfer_ok = 1;
	cfg.flags.ext_buffer = 1;
	cfg.flags.proxy_buffer = 1;
	cfg.callback = frame_cb;
	/* The interface that has the endpoint. */
	error = USB_ERR_NO_PIPE;
	for (i = 0; i < USB_IFACE_MAX && error == USB_ERR_NO_PIPE; i++) {
		if (usbd_get_iface(udev, i) == NULL)
			break;
		error = usbd_transfer_setup(udev, &i, p->xf, &cfg, 1, p,
		    &p->mtx);
	}
	if (error != 0) {
		sx_xunlock(&pipes_lock);
		x->error = error;
		sx_destroy(&p->lock);
		cv_destroy(&p->cv);
		mtx_destroy(&p->mtx);
		free(p, M_DEVBUF);
		return (NULL);
	}
	LIST_INSERT_HEAD(&pipes, p, link);
	sx_xunlock(&pipes_lock);
	return (p);
}

static int
xfer(struct bsdkern_usb_xfer *x)
{
	struct usb_device *udev;
	struct upipe *p;

	if ((udev = find(x->id)) == NULL)
		return (ENXIO);
	if (x->len > sizeof(x->data) ||
	    (x->type != UE_BULK && x->type != UE_INTERRUPT))
		return (EINVAL);
	x->actlen = 0;
	if ((p = pipe_get(udev, x)) == NULL)
		return (0);	/* x->error says why */
	sx_xlock(&p->lock);
	mtx_lock(&p->mtx);
	p->x = x;
	p->done = false;
	usbd_transfer_start(p->xf[0]);
	while (!p->done)
		cv_wait(&p->cv, &p->mtx);
	p->x = NULL;
	mtx_unlock(&p->mtx);
	sx_xunlock(&p->lock);
	return (0);
}

/* Blocks until the generation isn't *gen any more; gives the new one. */
static int
wait(uint32_t *gen)
{
	int error = 0;

	mtx_lock(&gen_mtx);
	while (generation == *gen && error == 0)
		error = msleep(&generation, &gen_mtx, PCATCH, "usbdgen", 0);
	*gen = generation;
	mtx_unlock(&gen_mtx);
	return (error == ERESTART ? EINTR : error);
}

/*
 * The USB core's devctl events: a device attached or detached
 * ("ugen=ugen<bus>.<address>", usbd's ID). A detach drops the device's
 * pipes before it is freed; both wake the clients waiting for a change.
 */
void	bsdkern_devctl(const char *, const char *, const char *, const char *);

void
bsdkern_devctl(const char *system, const char *subsystem, const char *type,
    const char *data)
{
	const char *u;
	u_int bus, addr;

	if (strcmp(system, "USB") != 0 || strcmp(subsystem, "DEVICE") != 0)
		return;
	if (strcmp(type, "DETACH") == 0 && data != NULL &&
	    (u = strstr(data, "ugen=ugen")) != NULL &&
	    sscanf(u, "ugen=ugen%u.%u", &bus, &addr) == 2)
		pipes_drop(bus << 8 | addr, NULL);
	if (strcmp(type, "ATTACH") == 0 || strcmp(type, "DETACH") == 0) {
		mtx_lock(&gen_mtx);
		generation++;
		wakeup(&generation);
		mtx_unlock(&gen_mtx);
	}
}

static int
usbd_ioctl(struct cdev *dev, u_long cmd, caddr_t data, int fflag,
    struct thread *td)
{
	switch (cmd) {
	case USBDIOCLIST:
		return (list((struct bsdkern_usb_list *)data));
	case USBDIOCCTRL:
		return (ctrl((struct bsdkern_usb_ctrl *)data));
	case USBDIOCXFER:
		return (xfer((struct bsdkern_usb_xfer *)data));
	case USBDIOCWAIT:
		return (wait((uint32_t *)data));
	default:
		return (ENOTTY);
	}
}

static struct cdevsw usbd_cdevsw = {
	.d_version =	D_VERSION,
	.d_ioctl =	usbd_ioctl,
	.d_name =	"usbd",
};

static void
serve_start(void *arg)
{
	(void)arg;
	make_dev(&usbd_cdevsw, 0, UID_ROOT, GID_WHEEL, 0600, "usbd");
}
SYSINIT(usbd_serve, SI_SUB_DRIVERS, SI_ORDER_ANY, serve_start, NULL);

/*
 * The USB client protocol, between usbd, which owns the host controllers
 * and the devices on them, and a server holding class drivers (netd's
 * if_cdce), through usbd's /dev/usbd (usbd's kern/serve.c; the client's
 * virtual host controller, usb/usbclient.c). FreeBSD's ioctl encoding:
 *
 * - USBDIOCLIST: the devices, each with its descriptor and speed;
 * - USBDIOCCTRL: a control request on a device, run by usbd's core
 *   (SET_ADDRESS is the client's own; SET_INTERFACE goes through usbd's
 *   usbd_set_alt_interface_index(), so both agree on the endpoints);
 * - USBDIOCXFER: one frame of a bulk or interrupt transfer on an endpoint;
 * - USBDIOCWAIT: waits until the devices change (an attach or a detach)
 *   from the generation the client knows, and gives the new one: the
 *   client's hot-plug.
 *
 * Each request is synchronous, on the serving thread; the client has a
 * thread per endpoint waiting in it. Data goes in the request (at most
 * BSDKERN_USB_DATA bytes: ioctl's encoding takes 8 KB at most).
 */
#ifndef _BSDKERN_USB_H_
#define	_BSDKERN_USB_H_

#include <sys/ioccom.h>

#define	BSDKERN_USB_DEVS	32
#define	BSDKERN_USB_DATA	4096

struct bsdkern_usb_dev {
	uint16_t	id;		/* the bus's unit << 8 | the address */
	uint8_t		speed;		/* enum usb_dev_speed */
	uint8_t		hub;		/* a hub: usbd's, not imported */
	uint8_t		desc[18];	/* struct usb_device_descriptor */
	uint8_t		pad[2];
};

struct bsdkern_usb_list {
	uint32_t		n;
	uint32_t		pad;
	struct bsdkern_usb_dev	dev[BSDKERN_USB_DEVS];
};

// ANCHOR: usb_protocol
struct bsdkern_usb_ctrl {
	uint16_t	id;
	uint16_t	actlen;		/* out */
	int32_t		error;		/* out: usb_error_t */
	uint8_t		req[8];		/* struct usb_device_request */
	uint32_t	timeout;	/* ms */
	uint32_t	pad;
	uint8_t		data[BSDKERN_USB_DATA];
};

struct bsdkern_usb_xfer {
	uint16_t	id;
	uint8_t		ep;		/* address, UE_DIR_IN for IN */
	uint8_t		type;		/* UE_BULK, UE_INTERRUPT */
	uint32_t	len;		/* in: OUT's length, IN's room */
	uint32_t	actlen;		/* out */
	int32_t		error;		/* out: usb_error_t */
	uint32_t	timeout;	/* ms, 0 for none */
	uint32_t	pad;
	uint8_t		data[BSDKERN_USB_DATA];
};

#define	USBDIOCLIST	_IOR('U', 200, struct bsdkern_usb_list)
#define	USBDIOCCTRL	_IOWR('U', 201, struct bsdkern_usb_ctrl)
#define	USBDIOCXFER	_IOWR('U', 202, struct bsdkern_usb_xfer)
#define	USBDIOCWAIT	_IOWR('U', 203, uint32_t)
// ANCHOR_END: usb_protocol

#endif

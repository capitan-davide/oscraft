/*
 * usbd's client protocol (bsdkern_usb.h) from a program: the devices usbd
 * holds, a control request on the first (the root hub's device
 * descriptor, which the controller's driver answers itself, so QEMU's
 * DWC2 needs no transfer on the wire), and a transfer on an endpoint the
 * device doesn't have (USB_ERR_NO_PIPE, 12), twice (the second on the
 * pipe usbd keeps, or not, for the endpoint), and the devices'
 * generation (hot-plug): the root hub's attach made it non-zero, so a
 * wait from the unknown one answers at once.
 */
#include <sys/types.h>
#include <sys/ioctl.h>

#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include "bsdkern_usb.h"

static struct bsdkern_usb_list l;
static struct bsdkern_usb_ctrl c;
static struct bsdkern_usb_xfer x;

int
main(void)
{
	int fd, r;
	uint32_t i, gen;

	fd = open("/dev/usbd", O_RDWR);
	if (fd < 0) {
		perror("usbd: open /dev/usbd");
		return (1);
	}
	r = ioctl(fd, USBDIOCLIST, &l);
	printf("usbd: list=%d n=%u\n", r, l.n);
	for (i = 0; i < l.n; i++)
		printf("usbd: device %d.%d speed=%d hub=%d vendor=%04x "
		    "product=%04x class=%d\n", l.dev[i].id >> 8,
		    l.dev[i].id & 0xff, l.dev[i].speed, l.dev[i].hub,
		    l.dev[i].desc[8] | l.dev[i].desc[9] << 8,
		    l.dev[i].desc[10] | l.dev[i].desc[11] << 8,
		    l.dev[i].desc[4]);
	if (l.n == 0)
		return (1);
	/* GET_DESCRIPTOR(DEVICE), 18 bytes. */
	c.id = l.dev[0].id;
	memcpy(c.req, "\x80\x06\x00\x01\x00\x00\x12\x00", 8);
	r = ioctl(fd, USBDIOCCTRL, &c);
	printf("usbd: ctrl=%d error=%d actlen=%d length=%d type=%d "
	    "same=%d\n", r, c.error, c.actlen, c.data[0], c.data[1],
	    memcmp(c.data, l.dev[0].desc, 18) == 0);
	x.id = l.dev[0].id;
	x.ep = 0x85;
	x.type = 2;	/* UE_BULK */
	x.len = 64;
	r = ioctl(fd, USBDIOCXFER, &x);
	printf("usbd: xfer=%d error=%d\n", r, x.error);
	x.error = 0;
	r = ioctl(fd, USBDIOCXFER, &x);
	printf("usbd: again=%d error=%d\n", r, x.error);
	gen = ~0u;
	r = ioctl(fd, USBDIOCWAIT, &gen);
	printf("usbd: wait=%d generation>0:%d\n", r, gen != 0 && gen != ~0u);
	return (0);
}

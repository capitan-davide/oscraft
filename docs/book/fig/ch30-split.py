from svg import Fig
f = Fig(800, 520)
# netd
f.box("u", 20, 16, 330, 300); f.text(185, 36, "netd", "h c"); f.text(185, 51, "the network stack's process", "s c")
f.box("g", 36, 62, 298, 34, 3); f.text(185, 77, "FreeBSD's network stack", "c"); f.text(185, 90, "sees an interface, ue0", "s c")
f.box("g", 36, 110, 298, 44, 3); f.text(185, 127, "class driver: if_cdce, on uether", "c"); f.text(185, 143, "if_cdce.ko, uether.ko", "ms c")
f.box("g", 36, 168, 298, 44, 3); f.text(185, 185, "FreeBSD's USB core", "c"); f.text(185, 201, "transfers, requests, the hub driver", "s c")
f.box("k", 36, 226, 298, 44, 3); f.text(185, 243, "client controller: usbcl", "b c"); f.text(185, 259, "a host controller with no hardware", "s c")
f.text(185, 292, "the last two are usb.ko", "s c")
f.line([(40, 280), (330, 280)], "life")
for y in (96, 154, 212): f.arrow([(185, y), (185, y + 13)])
# usbd
f.box("u", 450, 16, 330, 300); f.text(615, 36, "usbd", "h c"); f.text(615, 51, "owns the controllers and the devices", "s c")
f.box("k", 466, 62, 298, 44, 3); f.text(615, 79, "/dev/usbd", "b c"); f.text(615, 95, "list, control request, one frame, wait", "s c")
f.box("g", 466, 120, 298, 44, 3); f.text(615, 137, "FreeBSD's USB core", "c"); f.text(615, 153, "enumerates; attaches no class drivers", "s c")
f.box("g", 466, 178, 298, 44, 3); f.text(615, 195, "host controller driver: xhci", "c"); f.text(615, 211, "FreeBSD's xhci.c and xhci_pci.c", "s c")
f.box("k", 466, 236, 298, 44, 3); f.text(615, 253, "PCI client bus", "b c"); f.text(615, 269, "the driver's parent bus, backed by pcid", "s c")
for y in (106, 164, 222): f.arrow([(615, y), (615, y + 13)])
# the message path
f.line([(334, 248), (400, 248), (400, 198)], "lnk"); f.arrow([(400, 138), (400, 84), (464, 84)], "lnk", "ak")
f.num(400, 150, 1, "#2c5d9b"); f.text(400, 178, "ioctl():", "s c"); f.text(400, 192, "a message", "s c")
# pcid
f.box("u", 20, 350, 330, 70); f.text(185, 372, "pcid", "h c"); f.text(185, 390, "/dev/pci: configuration space,", "s c"); f.text(185, 404, "where a device's registers and interrupt are", "s c")
f.arrow([(540, 280), (540, 385), (352, 385)], "lnk", "ak"); f.num(540, 340, 2, "#2c5d9b"); f.text(528, 344, "set-up only", "s e")
# hardware
f.box("n", 450, 440, 330, 60); f.text(615, 462, "QEMU's xhci controller, on the PCIe bus", "c"); f.text(615, 480, "a USB Ethernet adapter on its port", "s c")
f.arrow([(650, 280), (650, 438)], "lnk", "ak"); f.num(650, 340, 3, "#2c5d9b"); f.text(662, 336, "registers,", "s"); f.text(662, 350, "mapped", "s")
f.arrow([(740, 438), (740, 282)], "lnr", "ar", True); f.num(740, 392, 4, "#aa2222"); f.text(730, 414, "interrupt", "s e red")
f.text(185, 462, "1: every USB transfer of the class driver", "s c")
f.text(185, 478, "2, 3: as any PCI driver in a server (Chapter 29)", "s c")
f.text(185, 494, "4: does not arrive yet on QEMU's virt", "s c red")
f.save("ch30-split.svg")

from svg import Fig
f = Fig(800, 430)
def proc(x, w, name, sub):
    f.box("frame", x, 14, w, 300); f.text(x + 12, 34, name, "h"); f.text(x + 12, 50, sub, "s")
def node(x, y, t, cls, s=None, w=150):
    f.box(cls, x, y, w, 30 if s else 24, 3); f.text(x + w / 2, y + (14 if s else 16), t, "ms c")
    if s: f.text(x + w / 2, y + 26, s, "s c")
def chain(x, items, w=150):
    y = 66
    for i, (t, cls, s) in enumerate(items):
        xx = x + i * 14
        node(xx, y, t, cls, s, w)
        if i: f.line([(xx - 7, y - 10), (xx - 7, y + 14), (xx, y + 14)])
        y += 40
proc(20, 280, "pcid", "owns the bus and configuration space")
chain(34, [("nexus0", "k", "bsdkern"), ("ofwbus0", "g", "the device tree's root"), ("pcib0", "g", "generic ECAM host bridge"),
           ("pci0", "g", "FreeBSD's PCI bus"), ("three functions", "w", "found and given addresses")], 200)
proc(320, 220, "netd", "holds the network driver")
chain(334, [("nexus0", "k", "bsdkern"), ("pci0", "k", "the client bus (pci.ko)"), ("virtio_pci0", "g", "virtio_pci.ko"), ("vtnet0", "g", "if_vtnet.ko")], 150)
proc(560, 220, "usbd", "holds the USB controller's driver")
chain(574, [("nexus0", "k", "bsdkern"), ("pci0", "k", "the client bus (built in)"), ("xhci0", "g", "FreeBSD's xhci"), ("usbus0", "g", "the USB bus")], 150)
f.box("k", 34, 270, 252, 34, 3); f.text(160, 285, "/dev/pci", "b c"); f.text(160, 299, "pci(4) requests, and PCIOCROUTE", "s c")
f.arrow([(430, 250), (430, 287), (288, 287)], "lnk", "ak"); f.arrow([(670, 250), (670, 295), (288, 295)], "lnk", "ak")
f.text(480, 282, "messages, at set-up", "s")
hw = [("host bridge", "1b36:0008", 34, 252), ("virtio network device", "1af4:1041", 334, 192), ("xhci controller", "1b36:000d", 574, 192)]
f.text(20, 348, "QEMU's virt machine", "h")
for t, i_d, x, w in hw:
    f.box("n", x, 360, w, 50); f.text(x + w / 2, 380, t, "c"); f.text(x + w / 2, 398, i_d, "ms c")
f.arrow([(250, 314), (250, 358)], "lng", "ag", False, True); f.text(258, 340, "ECAM", "s")
f.arrow([(510, 226), (510, 358)], "lng", "ag", False, True); f.text(516, 340, "BAR", "s")
f.arrow([(750, 226), (750, 358)], "lng", "ag", False, True); f.text(744, 340, "BAR", "s e")
f.save("ch29-servers.svg")

from svg import Fig
f = Fig(800, 440)
f.box("frame", 14, 8, 386, 390); f.text(24, 24, "netd", "s"); f.box("frame", 410, 8, 376, 390); f.text(420, 24, "pcid", "s")
cols = [("virtio_pci", "FreeBSD's driver", "g"), ("pci0", "the client bus", "k"), ("libc, libsys", "", "k"), ("devfs", "bsdkern", "k"), ("pci_user.c", "FreeBSD's", "g"), ("pcib0", "FreeBSD's bridge", "g")]
xs = [24 + i * 126 + (10 if i >= 3 else 0) for i in range(6)]
for (t, s, cls), x in zip(cols, xs):
    f.box(cls, x, 32, 116, 40); f.text(x + 58, 49, t, "b c"); f.text(x + 58, 64, s, "s c"); f.line([(x + 58, 72), (x + 58, 384)], "life")
c = [x + 58 for x in xs]
calls = ["pci_read_config(dev, reg, 4)", "ioctl(fd, PCIOCREAD, &io)", "a message: _IO_DEVCTL", "d_ioctl()", "PCIB_READ_CONFIG()"]
for i, t in enumerate(calls):
    y = 100 + i * 30
    f.arrow([(c[i], y), (c[i + 1], y)], "lnk" if i == 2 else "ln", "ak" if i == 2 else "a"); f.text((c[i] + c[i + 1]) / 2, y - 6, t, "ms c")
f.box("y", c[5] - 56, 232, 112, 30, 3); f.text(c[5], 251, "a load from ECAM", "s c")
rets = ["the value", "struct pci_io", "the reply", "0", "the value"]
for i, t in enumerate(rets):
    y = 286 + i * 22
    f.arrow([(c[5 - i], y), (c[4 - i], y)], "lng", "ag", True); f.text((c[5 - i] + c[4 - i]) / 2, y - 5, t, "s c")
f.text(400, 420, "Each arrow to the right is an ordinary function call, except the one between the servers. The answer returns the same way.", "s c")
f.save("ch29-config-read.svg")

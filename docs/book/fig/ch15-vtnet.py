from svg import Fig
f = Fig(800, 560)
cols = [("virtio network", "device (QEMU)", 80, "n"), ("GICv2", "", 215, "n"), ("kernel", "", 360, "k"), ("netd: the driver's", "interrupt thread", 540, "u"), ("pcid", "", 715, "u")]
X = {}
for a, b, x, cls in cols:
    f.box(cls, x - 65, 14, 130, 38)
    if b: f.text(x, 30, a, "s c b"); f.text(x, 44, b, "s c b")
    else: f.text(x, 38, a, "b c")
    f.line([(x, 52), (x, 540)], "life")
    X[a] = x
D, G, K, N, P = 80, 215, 360, 540, 715
def msg(x0, x1, y, t, cls="ln", m="a", dash=False, above=True, tx=None):
    f.arrow([(x0, y), (x1, y)], cls, m, dash)
    f.text(tx if tx is not None else (x0 + x1) / 2, y - 6 if above else y + 14, t, "s c")
def note(x, y, lines, cls="w", w=150):
    h = 10 + 13 * len(lines)
    f.box(cls, x - w / 2, y, w, h, 3)
    for i, l in enumerate(lines): f.text(x, y + 15 + i * 13, l, "s c")
# phase 1
f.text(14, 76, "Setting up, once", "h")
msg(N, P, 96, "ioctl(PCIOCROUTE): where is this device's interrupt?", "lnk", "ak")
msg(P, N, 122, "vector 32 + n", "lng", "ag", True)
msg(N, K, 150, "InterruptAttachThread(vector)", "lnk", "ak")
msg(K, G, 150, "unmask", "ln", "a")
msg(N, K, 180, "InterruptWait(UNMASK)", "lnk", "ak")
note(N, 190, ["blocked, waiting"], "y", 130)
f.line([(10, 232), (790, 232)], "life")
# phase 2
f.text(14, 254, "A packet arrives", "h")
msg(D, G, 280, "line active", "lnr", "ar")
msg(G, K, 304, "IRQ", "lnr", "ar")
note(K, 314, ["irq(): mask the vector,", "wake the thread;", "end of interrupt"], "k", 150)
msg(K - 75, G, 340, "mask", "ln", "a")
msg(K, N, 384, "InterruptWait() returns", "lng", "ag", tx=455)
note(N, 396, ["vtpci_intx_intr():", "reads the device's", "interrupt status"], "u", 150)
msg(N, D, 456, "reading the status register makes the device drop its line", "ln", "a", True)
note(N, 468, ["the packets go up", "FreeBSD's network stack"], "u", 150)
msg(N, K, 524, "InterruptWait(UNMASK)", "lnk", "ak")
msg(K, G, 524, "unmask", "ln", "a")
for n, y in [(1, 280), (2, 304), (3, 340), (4, 420), (5, 488), (6, 524)]: f.num(22, y, n)
f.save("ch15-vtnet.svg")

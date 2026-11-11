from svg import Fig
f = Fig(800, 420)
f.text(170, 18, "Physical addresses on QEMU's virt", "h c")
X, W = 120, 150
rows = [("w", 60, "nothing", "", "0x1_0000_0000"), ("w", 40, "nothing", "", "0xC000_0000"),
        ("g", 120, "RAM, 2 GiB", "ram; its free part: sysram", "0x4000_0000"), ("n", 80, "devices", "PL011 at 0x0900_0000 ...", "")]
y = 40
ys = []
f.text(X - 8, y + 4, "the CPU's limit", "s e"); f.line([(X - 6, y), (X, y)])
for cls, h, a, b, addr in rows:
    f.box(cls, X, y, W, h, 0)
    if b: f.text(X + W/2, y + h/2 - 2, a, "c"); f.text(X + W/2, y + h/2 + 13, b, "s c")
    else: f.text(X + W/2, y + h/2 + 4, a, "s c")
    ys.append(y); y += h
    if addr: f.text(X - 8, y + 4, addr, "ms e"); f.line([(X - 6, y), (X, y)])
    else: f.text(X - 8, y + 4, "0", "ms e"); f.line([(X - 6, y), (X, y)])
# brackets
def bracket(x, y0, y1, label, cls="ln"):
    f.line([(x - 6, y0), (x, y0), (x, y1), (x - 6, y1)], cls)
    f.text(x + 6, (y0 + y1) / 2 + 4, label, "ms")
bracket(X + W + 14, ys[1] + 2, y, "below4G")
f.line([(X + W + 86, 40), (X + W + 92, 40), (X + W + 92, y), (X + W + 86, y)]); f.text(X + W + 92, 34, "memory", "ms c")
f.text(190, y + 26, "Named ranges of the system page.", "s c"); f.text(190, y + 40, "There is no below1G: RAM starts at 1 GiB.", "s c")
# right: sequence
D, P = 550, 715
f.text(632, 18, "A driver gets a DMA buffer", "h c")
f.box("g", D - 70, 36, 140, 28); f.text(D, 55, "a driver in netd", "c")
f.box("k", P - 70, 36, 140, 28); f.text(P, 55, "process 1", "c")
f.line([(D, 64), (D, 390)], "life"); f.line([(P, 64), (P, 390)], "life")
def msg(y, n, label, sub, back):
    f.arrow([(D, y), (P - 2, y)], "lnk", "ak"); f.num(D - 16, y, n, "#2c5d9b")
    f.text((D + P) / 2, y - 6, label, "s c")
    f.arrow([(P, y + 26), (D + 2, y + 26)], "lng", "ag"); f.text((D + P) / 2, y + 20, back, "s c")
    f.text(D - 30, y + 4, sub[0], "s e")
    if len(sub) > 1: f.text(D - 30, y + 18, sub[1], "s e")
msg(100, 1, "open /memory/below4G", ["typed memory,", "to allocate"], "a descriptor")
msg(180, 2, "mmap(fd, len)", ["shared,", "uncached if asked"], "a virtual address")
msg(260, 3, "where is it?", ["_MEM_OFFSET"], "a physical address")
f.text(P + 8, 196, "one run of", "s"); f.text(P + 8, 209, "frames in", "s"); f.text(P + 8, 222, "the pool", "s")
f.box("n", D - 70, 330, 140, 40, 3); f.num(D - 86, 350, 4); f.text(D, 347, "writes the physical", "s c"); f.text(D, 361, "address to the device", "s c")
f.save("ch20-pools.svg")

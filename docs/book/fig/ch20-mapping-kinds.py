from svg import Fig
f = Fig(800, 360)
X, W = 40, 220
f.text(X + W/2, 24, "A process's virtual addresses", "h c")
rows = [("u", "anonymous", "a heap, a thread's stack"), ("u", "a file", "a library's code"), ("u", "a shared memory object", "a buffer two programs use"), ("u", "device memory", "a UART's registers")]
RX, RW = 520, 240
f.text(RX + RW/2, 24, "What is behind them", "h c")
right = [("k", "fresh page frames, zeroed", "nobody else has them"), ("y", "the bytes of the file", "its pages, or a copy of them"), ("y", "the object's page frames", "the same ones in every process"), ("n", "not RAM at all", "a physical address the device answers")]
y = 44
for i, ((c, a, b), (rc, ra, rb)) in enumerate(zip(rows, right)):
    f.box(c, X, y, W, 52, 2); f.text(X + W/2, y + 22, a, "c"); f.text(X + W/2, y + 38, b, "s c")
    f.box(rc, RX, y, RW, 52, 3); f.text(RX + RW/2, y + 22, ra, "c"); f.text(RX + RW/2, y + 38, rb, "s c")
    f.arrow([(X + W, y + 26), (RX - 3, y + 26)], "lnk", "ak")
    lab = ["mmap(MAP_ANON)", "mmap(fd of a file)", "mmap(fd of the object)", "mmap of a physical address"][i]
    f.text((X + W + RX) / 2, y + 18, lab, "ms c")
    if i < 3:
        f.box("w", X, y + 52, W, 22, 0); f.text(X + W/2, y + 67, "unmapped", "s c")
    y += 74
f.text(400, 346, "Every mapping is a range of pages with a protection: read, write, execute, or none.", "s c")
f.save("ch20-mapping-kinds.svg")

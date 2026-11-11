from svg import Fig
f = Fig(800, 400)
A, L, P = 110, 330, 620
for x, cls, t, w in [(A, "u", "the program", 150), (L, "g", "libc and libsys", 170), (P, "k", "process 1: memory manager", 250)]:
    f.box(cls, x - w/2, 30, w, 30); f.text(x, 50, t, "c"); f.line([(x, 60), (x, 370)], "life")
f.line([(A - 75, 22), (L + 85, 22)], "ln"); f.text((A + L) / 2, 16, "one thread of the calling process", "s c")
f.arrow([(A, 86), (L - 2, 86)], "ln", "a"); f.text((A + L) / 2, 80, "mmap(0, len, prot, flags, fd, off)", "ms c")
f.box("g", L - 95, 96, 190, 46, 3); f.num(L - 80, 119, 1, "#3e7b34"); f.text(L - 62, 115, "translates FreeBSD's flags", "s"); f.text(L - 62, 129, "and protection bits", "s")
f.arrow([(L, 160), (P - 2, 160)], "lnk", "ak"); f.text((L + P) / 2, 154, "_MEM_MAP", "ms c")
f.box("y", L - 60, 168, 120, 150, 3); f.text(L, 240, "REPLY-blocked", "s c")
steps = ["checks the arguments and abilities", "decides what the mapping is (Figure 20.5)", "allocates every page frame; zeroes or fills it", "writes the page-table entries", "records the region in the process's memory map"]
y = 168
for i, t in enumerate(steps):
    f.box("k", P - 150, y, 300, 26, 3); f.num(P - 136, y + 13, i + 2, "#2c5d9b"); f.text(P - 118, y + 17, t, "s")
    y += 30
f.arrow([(P, 330), (L + 62, 330)], "lng", "ag"); f.text((L + P) / 2 + 20, 324, "reply: the address", "s c")
f.arrow([(L, 346), (A + 2, 346)], "lng", "ag"); f.text((A + L) / 2, 340, "returns the address", "s c")
f.text(400, 388, "When mmap() returns, every page of the mapping has its frame. No later access can fault for lack of one.", "s c")
f.save("ch20-mmap-message.svg")

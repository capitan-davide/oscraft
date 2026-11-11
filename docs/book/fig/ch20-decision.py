from svg import Fig
f = Fig(800, 520)
f.text(400, 22, "What a _MEM_MAP request maps", "h c")
f.box("y", 20, 40, 150, 40); f.text(95, 57, "no descriptor", "b c"); f.text(95, 72, "fd is -1", "s c")
f.box("y", 20, 268, 150, 40); f.text(95, 285, "a descriptor", "b c"); f.text(95, 300, "fd names an open file", "s c")
CX, CW, RX, RW = 200, 250, 480, 300
def row(y, cond, sub, res, rsub, cls="k"):
    f.box("w", CX, y, CW, 40, 3); f.text(CX + CW/2, y + 17, cond, "c"); f.text(CX + CW/2, y + 32, sub, "s c")
    f.box(cls, RX, y, RW, 40, 3); f.text(RX + RW/2, y + 17, res, "c"); f.text(RX + RW/2, y + 32, rsub, "s c")
    f.arrow([(CX + CW, y + 20), (RX - 3, y + 20)], "lnk", "ak")
top = [("a physical address", "MAP_PHYS, not MAP_ANON", "device memory, as it is", "needs an ability; nothing is allocated", "n"),
       ("memory for a device", "MAP_PHYS with MAP_ANON, or uncached", "contiguous RAM below 4 GiB", "one run of zeroed frames"),
       ("anonymous, shared", "MAP_ANON | MAP_SHARED", "a new memory object, mapped shared", "a forked child shares its frames"),
       ("anonymous, private", "MAP_ANON | MAP_PRIVATE", "zeroed frames of the process's own", "a thread's stack gets a guard at its low end")]
y = 40
for i, r in enumerate(top):
    row(y, *r)
    f.line([(170 if i == 0 else 185, y + 20 if i == 0 else y + 20), (CX, y + 20)], "ln")
    y += 50
f.line([(185, 60), (185, y - 30)], "ln")
bot = [("a shared memory object", "opened under /dev/shmem", "its frames (shared) or a copy (private)", "", "y"),
       ("a typed memory pool", "opened to allocate, or not", "contiguous RAM from the pool,", "or the pool's memory at the offset"),
       ("a file of the boot image", "read-only mapping, page-aligned", "the image's own pages, in place", "shared by every process that maps them", "y"),
       ("a file of the boot image", "any other mapping", "a private copy of the file's bytes", "shared and writable: EACCES"),
       ("a file on another server", "", "ENODEV", "libsys then reads the file into anonymous memory", "r")]
y = 268
for i, r in enumerate(bot):
    c = r[4] if len(r) > 4 else "k"
    f.box("w", CX, y, CW, 40, 3)
    if r[1]: f.text(CX + CW/2, y + 17, r[0], "c"); f.text(CX + CW/2, y + 32, r[1], "s c")
    else: f.text(CX + CW/2, y + 24, r[0], "c")
    f.box(c, RX, y, RW, 40, 3)
    if r[3]: f.text(RX + RW/2, y + 17, r[2], "c"); f.text(RX + RW/2, y + 32, r[3], "s c")
    else: f.text(RX + RW/2, y + 24, r[2], "c")
    f.arrow([(CX + CW, y + 20), (RX - 3, y + 20)], "lnk", "ak")
    f.line([(170 if i == 0 else 185, y + 20), (CX, y + 20)], "ln")
    y += 50
f.line([(185, 288), (185, y - 30)], "ln")
f.save("ch20-decision.svg")

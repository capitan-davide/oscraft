from svg import Fig
f = Fig(820, 380)
f.text(20, 24, "physical memory: the boot image, where the startup left it", "h")
# the image strip
x0, y0 = 20, 40
parts = [("header", 80, "y"), ("directory: an entry per file", 230, "y"), ("data of /bin/sh", 170, "g"), ("data of /lib/libc.so.7", 190, "g"), ("...", 110, "y")]
x = x0; pos = {}
for name, w, cls in parts:
    f.box(cls, x, y0, w, 40, 0); f.text(x + w / 2, y0 + 24, name, "s c"); pos[name] = (x, w); x += w
dx, dw = pos["directory: an entry per file"]
# kernel
f.box("k", 20, 150, 300, 210)
f.text(36, 172, "process 1: the image file system", "b")
f.box("wk", 36, 184, 268, 50, 3); f.text(170, 204, "index, built once at boot:", "s c"); f.text(170, 221, "path -> directory entry", "ms c")
f.box("wk", 36, 246, 268, 50, 3); f.text(170, 266, "the whole image mapped read-only", "s c"); f.text(170, 283, "into the kernel", "s c")
f.box("y", 36, 308, 268, 40, 3); f.text(170, 325, "an open: the entry, a position,", "s c"); f.text(170, 340, "the open flags", "s c")
f.arrow([(290, 184), (dx + 190, y0 + 42)], "lnk", "ak")
# users
sx, sw = pos["data of /bin/sh"]
f.box("u", 400, 170, 180, 76); f.text(490, 190, "a program", "b c"); f.text(490, 208, "read(fd, buf, n)", "ms c"); f.text(490, 226, "gets a copy of the bytes", "s c")
f.box("u", 606, 170, 200, 76); f.text(706, 190, "two processes running sh", "b c"); f.text(710, 208, "mmap() of its code", "ms c"); f.text(710, 226, "the image's own pages", "s c")
f.arrow([(sx + 50, y0 + 42), (470, 168)], "ln", "a", dash=True)
f.arrow([(sx + 120, y0 + 42), (700, 168)], "lng", "ag")
f.text(372, 128, "copied", "s e"); f.text(600, 128, "shared, not copied", "s green")
f.box("r", 400, 280, 400, 60, 3)
f.text(600, 302, "Nothing can be changed: opening for writing, creating", "s c")
f.text(600, 318, "and removing fail. Other servers' names are added", "s c")
f.text(600, 334, "to its directory listings.", "s c")
f.save("ch22-imagefs.svg")

from svg import Fig
f = Fig(800, 500)
X, W = 250, 300
f.text(X + W/2, 22, "A process's address space after loading", "h c")
rows = [  # cls, h, title, sub, address at top edge
 ("y", 36, "the system page", "read-only, the same in every process", "0x80_0000_0000"),
 ("w", 16, "", "", "0x7f_ffff_d000"),
 ("u", 60, "the main thread's stack", "512 KiB, every page backed at load time", "0x7f_ffff_c000"),
 ("r", 20, "guard page: never mapped", "", "0x7f_fff7_c000"),
 ("w", 40, "...", "", ""),
 ("g", 54, "mmap() without an address", "shared libraries, heap, thread stacks", ""),
 ("w", 30, "...", "", "0x10_0000_0000  (64 GiB)"),
 ("g", 44, "ld-elf.so.1", "only for a dynamically linked program", ""),
 ("w", 30, "...", "", "0x4_0000_0000  (16 GiB)"),
 ("k", 54, "the program", "its segments, if position-independent", ""),
 ("w", 40, "a program linked for fixed", "addresses lies where it says", "0x1_0000_0000  (4 GiB)"),
]
y = 40
for cls, h, a, b, addr in rows:
    f.box(cls, X, y, W, h, 0)
    if a and b: f.text(X + W/2, y + h/2 - 2, a, "c"); f.text(X + W/2, y + h/2 + 12, b, "s c")
    elif a: f.text(X + W/2, y + h/2 + 4, a, "c" if h > 22 else "s c")
    if addr: f.text(X - 10, y + 4, addr, "ms e"); f.line([(X - 6, y), (X, y)])
    y += h
f.text(X - 10, y + 4, "0", "ms e"); f.line([(X - 6, y), (X, y)])
# initial stack marker
sy = 40 + 36 + 16
f.box("wu", X + W + 24, sy, 190, 44, 3); f.text(X + W + 119, sy + 18, "at its top: the initial stack", "s c"); f.text(X + W + 119, sy + 32, "(Figure 19.6)", "s c")
f.line([(X + W, sy + 8), (X + W + 24, sy + 8)], "ln", True)
f.text(X + W + 24, 44, "512 GiB: the end of user space", "s")
f.text(X + W + 24, 338, "entry point for a", "s"); f.text(X + W + 24, 352, "dynamic program", "s")
f.text(X + W + 24, 420, "entry point for a", "s"); f.text(X + W + 24, 434, "static program", "s")
f.text(400, 486, "Not to scale. Addresses are those of QEMU's virt; high addresses at the top.", "s c")
f.save("ch19-address-space.svg")

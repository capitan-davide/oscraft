from svg import Fig
f = Fig(800, 520)
X, W = 250, 300
f.text(X + W/2, 22, "The top of a new FreeBSD program's stack", "h c")
rows = [
 ("y", 40, "struct ps_strings", "argv, argc, envp, envc", "0x7f_ffff_c000  the stack's top"),
 ("w", 26, "the executable's path", "", ""),
 ("w", 26, "64 bytes for the stack protector", "", ""),
 ("w", 26, "the page sizes: [4096]", "", ""),
 ("u", 40, "argument and environment strings", "\"hello\\0one\\0two\\0PATH=...\\0\"", ""),
 ("n", 18, "padding to 16 bytes", "", ""),
 ("k", 92, "", "", ""),
 ("u", 24, "NULL", "", ""),
 ("u", 24, "envp[0] ... envp[envc-1]", "", ""),
 ("u", 24, "NULL", "", ""),
 ("u", 24, "argv[0] ... argv[argc-1]", "", ""),
 ("g", 28, "argc", "", ""),
]
y = 40
ys = []
for cls, h, a, b, addr in rows:
    f.box(cls, X, y, W, h, 0)
    if a and b: f.text(X + W/2, y + h/2 - 2, a, "c"); f.text(X + W/2, y + h/2 + 12, b, "ms c")
    elif a: f.text(X + W/2, y + h/2 + 4, a, "c" if h > 20 else "s c")
    if addr: f.text(X - 10, y + 4, addr, "ms e"); f.line([(X - 6, y), (X, y)])
    ys.append(y); y += h
# auxv contents
ay = ys[6]
f.text(X + W/2, ay + 18, "the auxiliary vector", "c")
f.text(X + W/2, ay + 36, "(AT_PHDR, a) (AT_PHNUM, n) (AT_ENTRY, e)", "ms c")
f.text(X + W/2, ay + 50, "(AT_BASE, b) (AT_EXECPATH, p) ...", "ms c")
f.text(X + W/2, ay + 64, "(AT_PS_STRINGS, s) ... (AT_NULL, 0)", "ms c")
f.text(X + W/2, ay + 82, "pairs of 64-bit words: a type and a value", "s c")
# sp
f.arrow([(X - 70, y - 14), (X - 2, y - 14)], "lng", "ag")
f.text(X - 76, y - 17, "sp at entry,", "s e"); f.text(X - 76, y - 4, "16-byte aligned;", "s e"); f.text(X - 76, y + 9, "x0 holds the same address", "s e")
f.line([(X, y), (X, y + 26)], "life"); f.line([(X + W, y), (X + W, y + 26)], "life")
f.text(X + W/2, y + 20, "the rest of the stack: free, 512 KiB in all", "s c")
# pointers on the right
RX = X + W
def ptr(y0, y1, off, label, cls="ln", m="a"):
    f.arrow([(RX, y0), (RX + off, y0), (RX + off, y1), (RX + 2, y1)], cls, m, True)
ptr(ys[10] + 12, ys[4] + 28, 28, "")
ptr(ys[8] + 12, ys[4] + 14, 44, "")
f.text(RX + 52, ys[6] + 30, "argv and envp", "s"); f.text(RX + 52, ys[6] + 44, "point at the strings", "s")
f.arrow([(RX, ay + 60), (RX + 190, ay + 60), (RX + 190, ys[0] + 20), (RX + 2, ys[0] + 20)], "lnk", "ak", True)
f.text(RX + 184, ys[1] + 16, "AT_PS_STRINGS", "ms e blue")
f.arrow([(X, ys[0] + 28), (X - 30, ys[0] + 28), (X - 30, ys[10] + 12), (X - 2, ys[10] + 12)], "ln", "a", True)
f.arrow([(X - 30, ys[8] + 12), (X - 2, ys[8] + 12)], "ln", "a", True)
f.text(X - 38, ys[3] + 10, "ps_strings finds", "s e"); f.text(X - 38, ys[3] + 24, "argv and envp again,", "s e"); f.text(X - 38, ys[3] + 38, "at any later time", "s e")
f.text(400, 508, "High addresses at the top. Process 1 writes all of this before the first thread runs.", "s c")
f.save("ch19-initial-stack.svg")

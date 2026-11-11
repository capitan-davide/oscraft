from svg import Fig
f = Fig(800, 400)
# the ELF identification bytes
f.text(400, 22, "The first 16 bytes of the program file (the ELF identification)", "h c")
names = ["7f", "'E'", "'L'", "'F'", "class", "data", "version", "OSABI", "ABI ver."]
x = 120
for i, n in enumerate(names):
    w = 62
    cls = "y" if i == 7 else "w"
    f.box(cls, x, 34, w, 30, 2); f.text(x + w / 2, 54, n, "ms c"); f.text(x + w / 2, 78, str(i), "s c")
    x += w
f.box("w", x, 34, 62, 30, 2); f.text(x + 31, 54, "padding", "ms c"); f.text(x + 31, 78, "9..15", "s c")
bx = 120 + 7 * 62 + 31
f.arrow([(bx, 84), (bx, 104)])
f.box("k", bx - 150, 106, 300, 34); f.text(bx, 127, "Personality::from_osabi(byte 7)", "m c")
L, R = 210, 590
f.arrow([(bx - 80, 140), (L, 168)], "lng", "ag"); f.text(L + 60, 150, "9 (FreeBSD)", "s e green")
f.arrow([(bx + 60, 140), (R, 168)], "lnr", "ar"); f.text(R + 14, 150, "anything else", "s red")
def col(cx, cls, title, rows):
    f.box(cls, cx - 180, 170, 360, 30); f.text(cx, 190, title, "b c")
    y = 206
    for k, v in rows:
        f.box("w", cx - 180, y, 360, 34, 2)
        f.text(cx - 172, y + 14, k, "s"); f.text(cx - 172, y + 28, v, "ms")
        y += 38
col(L, "g", "Native: a FreeBSD process", [
    ("initial stack", "FreeBSD's: ps_strings, argc, argv, envp, auxv"),
    ("thread pointer at entry", "0: libc makes its own thread-local storage"),
    ("dynamic loader", "ld-elf.so.1"),
    ("data from the kernel and process 1", "as they are"),
])
col(R, "r", "Qnx: a QNX process", [
    ("initial stack", "QNX's: thread-local block, argc, argv, envp, auxv"),
    ("thread pointer at entry", "the block the kernel built on the stack"),
    ("dynamic loader", "ldqnx-64.so.2"),
    ("data from the kernel and process 1", "converted to QNX's numbers and layouts"),
])
f.text(400, 388, "Chosen once, when the program is loaded; a child made by fork() has its parent's.", "s c")
f.save("ch32-personality.svg")

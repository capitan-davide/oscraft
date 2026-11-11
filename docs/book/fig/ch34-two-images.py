from svg import Fig
f = Fig(800, 400)
def stack(x, title, cmd, rows):
    f.text(x + 170, 24, title, "h c"); f.text(x + 170, 42, cmd, "m c")
    y = 54
    for cls, a, b, h in rows:
        f.box(cls, x, y, 340, h); f.text(x + 170, y + h / 2 - (3 if b else -4), a, "c")
        if b: f.text(x + 170, y + h / 2 + 13, b, "s c")
        y += h + 6
stack(30, "The reference image", "make qnx-test", [
    ("u", "the QNX test programs", "tests/qnx, built with the SDP's compiler", 46),
    ("r", "QNX's shell on QNX's serial driver", "the console the tests are typed into", 46),
    ("r", "QNX's C library, loader, pipe manager, tools", None, 40),
    ("r", "procnto, QNX's kernel", "from the SDP", 60),
    ("k", "OsCraft's startup for virt", None, 36),
    ("n", "QEMU's virt machine", None, 36),
])
stack(430, "OsCraft's test image", "make test QNX=1", [
    ("u", "the QNX test programs", "the same files", 46),
    ("r", "QNX's shell on QNX's serial driver", "the same files", 46),
    ("r", "QNX's C library, loader and tools; qnxd", "the native servers and FreeBSD's userland below", 40),
    ("k", "OsCraft's kernel and process 1", None, 60),
    ("k", "OsCraft's startup for virt", None, 36),
    ("n", "QEMU's virt machine", None, 36),
])
for y in (77, 129, 288, 330):
    f.line([(372, y), (428, y)], "lng"); 
f.text(400, 71, "same", "s c green"); f.text(400, 123, "same", "s c green"); f.text(400, 282, "same", "s c green"); f.text(400, 324, "same", "s c green")
f.line([(372, 234), (428, 234)], "lnr"); f.text(400, 228, "differs", "s c red")
f.text(400, 380, "Only the kernel, and what the kernel brings with it, differs between the two runs.", "s c")
f.save("ch34-two-images.svg")

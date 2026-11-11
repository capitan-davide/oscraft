from svg import Fig
f = Fig(820, 360)
f.text(120, 24, "connections", "h c"); f.text(120, 40, "what the kernel knows", "s c")
f.text(400, 24, "the map", "h c"); f.text(400, 40, "(process, connection) -> context", "s c")
f.text(690, 24, "open-file contexts", "h c"); f.text(690, 40, "shared, each with its own lock", "s c")
def proc(y, name, fds):
    h = 30 + 30 * len(fds)
    f.box("u", 20, y, 200, h); f.text(120, y + 18, name, "s c")
    for i, fd in enumerate(fds):
        f.box("wu", 40, y + 26 + i * 30, 160, 24, 3); f.text(120, y + 42 + i * 30, fd, "m c")
proc(56, "process 12", ["fd 3", "fd 4 = dup(3)"])
proc(160, "process 13, forked from 12", ["fd 3"])
proc(236, "process 20", ["fd 0", "fd 5"])
keys = [("(12, 3)", 0), ("(12, 4)", 0), ("(13, 3)", 0), ("(20, 0)", 1), ("(20, 5)", 2)]
ys = [94, 124, 198, 274, 304]
f.box("k", 320, 56, 160, 284)
ctx = [(100, "Image", "bin/sh, position 4096"), (210, "Text", "the console's open flags"), (300, "Proc", "/proc/20/cmdline, position 0")]
for y, kind, what in ctx:
    f.box("y", 580, y - 24, 220, 48); f.text(690, y - 4, kind, "m c"); f.text(690, y + 13, what, "s c")
for (k, c), y in zip(keys, ys):
    f.box("wk", 336, y - 12, 128, 24, 3); f.text(400, y + 4, k, "m c")
    f.arrow([(200, y), (334, y)], "lnk", "ak")
    f.arrow([(464, y), (578, ctx[c][0] + (y - ys[1]) * 0.12 if c == 0 else ctx[c][0])])
f.save("ch22-ocbs.svg")

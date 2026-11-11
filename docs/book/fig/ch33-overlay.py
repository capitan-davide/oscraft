from svg import Fig
f = Fig(800, 380)
# left: the tree
f.text(150, 24, "One file system", "h c")
tree = [(0, "/", "w"), (1, "bin/sh", "g"), (1, "lib/libc.so.7", "g"), (1, "dev/null", "g"), (1, "tmp/", "g"),
        (1, "compat/qnx/", "r"), (2, "bin/sh", "r"), (2, "bin/ksh", "r"), (2, "lib/libc.so.6", "r")]
y = 40
pos = {}
for d, name, cls in tree:
    x = 30 + d * 36
    f.box(cls, x, y, 170, 26, 2); f.text(x + 10, y + 17, name, "m")
    pos[(d, name)] = y + 13
    y += 32
f.text(150, y + 14, "green: FreeBSD's files    red: QNX's, from the SDP", "s c")
# right: three lookups
X = 330
def look(y, who, cls, path, steps, result, rcls):
    f.box(cls, X, y, 130, 44); f.text(X + 65, y + 19, who, "b c"); f.text(X + 65, y + 35, 'opens "' + path + '"', "ms c")
    f.arrow([(X + 130, y + 22), (X + 158, y + 22)])
    f.box("k", X + 160, y, 150, 44)
    for i, t in enumerate(steps): f.text(X + 235, y + 19 + i * 15, t, "s c")
    f.arrow([(X + 310, y + 22), (X + 338, y + 22)])
    f.box(rcls, X + 340, y, 120, 44); f.text(X + 400, y + 19, "gets", "s c"); f.text(X + 400, y + 35, result, "ms c")
f.text(X + 230, 24, "Three lookups", "h c")
look(50, "a native process", "g", "/bin/sh", ["no overlay:", "the path as it is"], "/bin/sh", "g")
look(140, "a QNX process", "r", "/bin/sh", ["/compat/qnx/bin/sh", "exists: use it"], "QNX's sh", "r")
look(230, "a QNX process", "r", "/dev/null", ["/compat/qnx/dev/null", "does not exist: as it is"], "/dev/null", "g")
f.text(X + 230, 310, "The overlay is tried for every path a QNX process names,", "s c")
f.text(X + 230, 325, "before the path manager looks for the server that owns it.", "s c")
f.save("ch33-overlay.svg")

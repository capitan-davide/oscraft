from svg import Fig
f = Fig(800, 330)
f.text(400, 24, "Who converts, for each pair of client and server", "h c")
CX, CW, RY, RH = [250, 520], 250, [80, 190], 100
f.box("g", CX[0], 40, CW, 30); f.text(CX[0] + CW / 2, 60, "native server (FreeBSD's layouts)", "b c")
f.box("r", CX[1], 40, CW, 30); f.text(CX[1] + CW / 2, 60, "QNX server (QNX's layouts)", "b c")
f.box("g", 30, RY[0], 210, RH); f.text(135, RY[0] + 46, "native client", "b c"); f.text(135, RY[0] + 62, "FreeBSD's C library on libsys", "s c")
f.box("r", 30, RY[1], 210, RH); f.text(135, RY[1] + 46, "QNX client", "b c"); f.text(135, RY[1] + 62, "QNX's C library", "s c")
cells = [
    (0, 0, "w", "nobody", ["the message goes straight", "to the server"]),
    (1, 0, "k", "libsys, in the client", ["the kernel tells it the server", "is a QNX program; no extra hop"]),
    (0, 1, "y", "qnxd, a process in between", ["the kernel routes the message to it;", "one extra hop"]),
    (1, 1, "w", "nobody", ["the message goes straight", "to the server"]),
]
for c, r, cls, head, lines in cells:
    x, y = CX[c], RY[r]
    f.box(cls, x, y, CW, RH)
    f.text(x + CW / 2, y + 36, head, "b c")
    for i, t in enumerate(lines): f.text(x + CW / 2, y + 58 + i * 15, t, "s c")
f.text(400, 316, "Process 1 is outside the table: it answers each client in the client's own layouts.", "s c")
f.save("ch33-matrix.svg")

from svg import Fig
f = Fig(800, 440)
P, M, T = 130, 400, 670
for x, cls, t, sub in [(P, "u", "a program", "libc and libsys"), (M, "k", "process 1", "the pathname manager"), (T, "u", "tmpfs", "owns /tmp")]:
    f.box(cls, x - 95, 16, 190, 44); f.text(x, 35, t, "b c"); f.text(x, 51, sub, "s c")
    f.line([(x, 60), (x, 420)], "life")
f.text(P, 84, 'fd = open("/tmp/notes", O_RDONLY)', "ms c")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    mid = (x0 + x1) / 2
    f.num(min(x0, x1) + 22, y - 13, n, col); f.text(min(x0, x1) + 38, y - 9, text, "s")
step(1, 118, P, M - 2, "who owns /tmp/notes?")
step(2, 152, M, P + 2, "tmpfs does: its process and channel", "lng", "ag", "#3e7b34")
f.box("wu", P - 90, 164, 180, 30, 3); f.text(P, 183, "connect to tmpfs's channel", "s c")
step(3, 222, P, T - 2, "open notes, to read")
f.box("wu", T - 90, 230, 180, 30, 3); f.text(T, 249, "finds the file, checks access", "s c")
step(4, 286, T, P + 2, "done", "lng", "ag", "#3e7b34")
f.box("g", P - 90, 296, 180, 30, 3); f.text(P, 315, "the connection is the fd", "s c")
f.text(P, 350, "n = read(fd, buf, 4096)", "ms c")
step(5, 380, P, T - 2, "read 4096 bytes")
step(6, 408, T, P + 2, "the bytes", "lng", "ag", "#3e7b34")
f.box("y", M - 95, 300, 190, 56, 3); f.text(M, 320, "not involved any more:", "s c"); f.text(M, 335, "the descriptor already", "s c"); f.text(M, 350, "leads to the server", "s c")
f.save("ch02-open-read.svg")

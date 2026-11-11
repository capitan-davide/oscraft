from svg import Fig
f = Fig(820, 470)
P, M, I, T = 120, 350, 560, 730
for x, cls, t, sub in [(P, "u", "a program", "libsys"), (M, "k", "process 1", "pathname manager"), (I, "k", "process 1", "image file system"), (T, "u", "tmpfs", "owns /tmp")]:
    f.box(cls, x - 80, 16, 160, 44); f.text(x, 35, t, "b c"); f.text(x, 51, sub, "s c")
    f.line([(x, 60), (x, 455)], "life")
f.text(P - 100, 84, 'open("/usr/tmp/notes")', "ms")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    f.num(min(x0, x1) + 22, y - 13, n, col); f.text(min(x0, x1) + 38, y - 9, text, "s")
step(1, 120, P, M - 2, "who owns /usr/tmp/notes?")
step(2, 154, M, P + 2, "the image file system", "lng", "ag", "#3e7b34")
step(3, 196, P, I - 2, "open usr/tmp/notes")
f.box("wk", I - 100, 204, 200, 30, 3); f.text(I, 223, "usr/tmp is a link to /tmp", "s c")
step(4, 262, I, P + 2, "a link: the path is /tmp/notes, no servers", "lnr", "ar", "#aa2222")
f.box("y", P - 100, 272, 200, 44, 3); f.text(P, 290, "start again with the new path", "s c"); f.text(P, 305, "(at most 16 times, then ELOOP)", "s c")
step(5, 346, P, M - 2, "who owns /tmp/notes?")
step(6, 380, M, P + 2, "tmpfs, then the image file system", "lng", "ag", "#3e7b34")
step(7, 414, P, T - 2, "open notes")
step(8, 448, T, P + 2, "done", "lng", "ag", "#3e7b34")
f.save("ch21-symlink.svg")

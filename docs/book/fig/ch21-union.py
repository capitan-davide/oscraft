from svg import Fig
f = Fig(800, 356)
C, B, A = 140, 430, 680
for x, cls, t, sub in [(C, "u", "a program", "its C library"), (B, "u", "server B", "registered /etc"), (A, "u", "server A", "registered /")]:
    f.box(cls, x - 95, 16, 190, 44); f.text(x, 35, t, "b c"); f.text(x, 51, sub, "s c")
    f.line([(x, 60), (x, 340)], "life")
f.box("y", C - 110, 76, 220, 62, 3)
f.text(C, 94, 'open("/etc/hosts"): who to ask', "s c")
f.text(C - 96, 112, "1. server B, owns /etc", "ms")
f.text(C - 96, 128, "2. server A, owns /", "ms")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    f.num(min(x0, x1) + 22, y - 13, n, col); f.text(min(x0, x1) + 38, y - 9, text, "s")
step(1, 172, C, B - 2, 'open "hosts"')
f.box("wu", B - 80, 180, 160, 28, 3); f.text(B, 198, "has passwd, group only", "s c")
step(2, 236, B, C + 2, "no such file", "lnr", "ar", "#aa2222")
step(3, 270, C, A - 2, 'open "etc/hosts"')
f.box("wu", A - 80, 278, 160, 28, 3); f.text(A, 296, "has etc/hosts", "s c")
step(4, 334, A, C + 2, "done: this connection is the file", "lng", "ag", "#3e7b34")
f.save("ch21-union.svg")

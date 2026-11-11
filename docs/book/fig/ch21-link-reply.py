from svg import Fig
f = Fig(820, 330)
f.text(30, 26, 'The answer to "who owns /var/run/log?"', "h")
f.text(646, 26, "the program then asks", "h")
X, W = 30, 560
f.box("y", X, 40, W, 34, 3)
f.text(X + 12, 61, "header", "b")
f.text(X + 80, 61, "3 servers follow; the caller's umask; the length of the path", "s")
cols = [(X + 12, "server"), (X + 190, "channel"), (X + 290, "handle"), (X + 390, "prefix length")]
for x, t in cols:
    f.text(x, 98, t, "s")
rows = [
    ("syslogd's process", "its channel", "its handle", "12", "u", "syslogd", '""', "the name itself"),
    ("tmpfs's process", "its channel", "its handle", "5", "u", "tmpfs", '"run/log"', ""),
    ("process 1", "1", "1: the image", "1", "k", "process 1", '"var/run/log"', ""),
]
for i, (p, c, h, n, cls, who, rest, note) in enumerate(rows):
    y = 106 + i * 42
    f.box(cls, X, y, W, 34, 3)
    f.text(X + 12, y + 21, p, "")
    f.text(X + 190, y + 21, c, "s")
    f.text(X + 290, y + 21, h, "s")
    f.text(X + 390, y + 21, n, "m")
    f.num(X + W - 20, y + 17, i + 1)
    f.arrow([(X + W + 6, y + 17), (636, y + 17)])
    f.text(646, y + 14, who + ", to open", "s")
    f.text(646, y + 29, rest, "m")
y = 106 + 3 * 42
f.box("y", X, y, W, 34, 3)
f.text(X + 12, y + 21, "path", "b")
f.text(X + 80, y + 21, "/var/run/log", "m")
f.text(X + 190, y + 21, "absolute: the working directory, . and .. resolved", "s")
# the prefix lengths, drawn on the path
y2 = y + 60
f.text(X + 12, y2 + 4, "prefix lengths:", "s")
px, cw = X + 130, 13
for i, ch in enumerate("/var/run/log"):
    f.box("w", px + i * cw, y2 - 12, cw, 22, 0); f.text(px + i * cw + cw / 2, y2 + 4, ch, "m c")
for n, lab, dy in [(1, "1", 0), (5, "5", 0), (12, "12", 0)]:
    x = px + n * cw
    f.line([(x, y2 - 18), (x, y2 + 16)], "lnr")
    f.text(x, y2 + 30, lab, "s c red")
f.text(px + 12 * cw + 24, y2 + 4, "each server is sent what follows its own prefix", "s")
f.save("ch21-link-reply.svg")

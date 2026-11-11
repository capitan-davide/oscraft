from svg import Fig
f = Fig(820, 372)
# top: one answer, two forms
f.text(20, 24, "One answer, in the caller's form", "h")
f.box("k", 20, 40, 200, 70)
f.text(120, 62, "a built-in manager", "b c"); f.text(120, 80, "fills FreeBSD's struct stat", "s c"); f.text(120, 96, "(Stat, 224 bytes)", "s c")
f.box("wk", 290, 52, 150, 46, 3); f.text(365, 72, "who is asking?", "s c"); f.text(365, 88, "encode(client)", "ms c")
f.arrow([(220, 75), (288, 75)], "lnk", "ak")
f.box("g", 520, 36, 280, 40, 3); f.text(660, 53, "a native process: the 224 bytes as they are", "s c"); f.text(660, 68, "reply status 0x4642", "ms c")
f.box("r", 520, 86, 280, 40, 3); f.text(660, 103, "a QNX process: converted to QNX's struct", "s c"); f.text(660, 118, "(Part VI)", "s c")
f.arrow([(440, 68), (518, 56)], "lng", "ag"); f.arrow([(440, 82), (518, 104)], "lnr", "ar")
# bottom: a dirent record
f.text(20, 170, "A directory is read as records: FreeBSD's struct dirent", "h")
x0, y0, u = 20, 190, 16
fields = [("d_fileno", 8, "inode number"), ("d_off", 8, "position"), ("d_reclen", 2, ""), ("d_type", 1, ""), ("", 1, ""), ("d_namlen", 2, ""), ("", 2, ""), ("d_name", 8, "the name, a 0 byte, padding")]
x = x0
for name, n, note in fields:
    w = n * u * (2.2 if name == "d_name" else 1) if n >= 8 else max(n * u, 16) * 2
    cls = "y" if name else "n"
    f.box(cls, x, y0, w, 44, 0)
    if n >= 8:
        f.text(x + w / 2, y0 + 20, name, "m c"); f.text(x + w / 2, y0 + 36, note, "s c")
    x += w
# narrow fields: labels below with leaders
lab = [("d_reclen: this record's length", 0), ("d_type: 0, unknown", 1), ("d_namlen: the name's length", 3)]
xs = []
x = x0
for name, n, note in fields:
    w = n * u * (2.2 if name == "d_name" else 1) if n >= 8 else max(n * u, 16) * 2
    xs.append((x, w)); x += w
for (t, idx), ly in zip([(lab[0][0], 2), (lab[1][0], 3), (lab[2][0], 5)], [298, 280, 262]):
    cx = xs[idx][0] + xs[idx][1] / 2
    f.line([(cx, y0 + 44), (cx, ly - 4), (cx + 16, ly - 4)], "ln")
    f.text(cx + 20, ly, t, "s")
for off, idx in [("0", 0), ("8", 1), ("16", 2), ("24", 7)]:
    f.text(xs[idx][0] + 2, y0 - 5, off, "s")
f.text(20, 340, "Each record is padded to a multiple of 8 bytes. A read returns as many whole records as fit in the caller's buffer,", "s")
f.text(20, 356, "and the open's position counts the names already returned.", "s")
f.save("ch22-stat.svg")

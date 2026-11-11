from svg import Fig
f = Fig(800, 420)
f.text(400, 24, "A FAT volume, from block 0", "h c")
parts = [("y", 20, 90, "boot sector", "sizes, counts"), ("k", 110, 130, "FAT", "one entry per cluster"), ("k", 240, 130, "FAT, second copy", "kept identical"), ("y", 370, 110, "root directory", "fixed size (FAT12/16)"), ("g", 480, 300, "data area: clusters 2, 3, 4, ...", "files and the other directories")]
for c, x, w, t, s in parts:
    f.box(c, x, 36, w, 50, 2); f.text(x + w / 2, 58, t, "c"); f.text(x + w / 2, 74, s, "s c")
# directory entry
f.text(20, 126, "a directory entry, 32 bytes", "b")
ex = 20
for w, t in [(150, "name: TODO    TXT"), (90, "attributes"), (120, "time, date"), (130, "first cluster: 5"), (130, "size: 1300")]:
    f.box("w", ex, 136, w, 30, 2); f.text(ex + w / 2, 155, t, "ms c"); ex += w
# FAT table
f.text(20, 214, "the FAT", "b"); f.text(20, 230, "entry n says which", "s"); f.text(20, 244, "cluster follows n", "s")
vals = [("2", "0"), ("3", "0"), ("4", "0"), ("5", "6"), ("6", "9"), ("7", "0"), ("8", "0"), ("9", "end"), ("10", "0")]
x0 = 170
for i, (n, v) in enumerate(vals):
    x = x0 + i * 66
    used = v != "0"
    f.text(x + 30, 204, n, "s c"); f.box("g" if used else "w", x, 210, 60, 34, 2); f.text(x + 30, 231, v if used else "free", "m c" if used else "s c")
cx = lambda n: x0 + (n - 2) * 66 + 30
f.arrow([(445, 166), (445, 180), (cx(5), 180), (cx(5), 192)], "lnk", "ak")
f.arrow([(cx(5), 244), (cx(5), 262), (cx(6), 262), (cx(6), 246)], "lng", "ag")
f.arrow([(cx(6) + 12, 244), (cx(6) + 12, 272), (cx(9), 272), (cx(9), 246)], "lng", "ag")
# data clusters
f.text(20, 330, "the data area", "b"); f.text(20, 346, "a cluster is one or", "s"); f.text(20, 360, "more blocks", "s")
for i, (n, v) in enumerate(vals):
    x = x0 + i * 66
    t = {"5": "bytes 0…", "6": "…", "9": "…1299"}.get(n)
    f.box("g" if t else "n", x, 318, 60, 44, 2); f.text(x + 30, 336, "cluster " + n, "s c")
    if t: f.text(x + 30, 352, t, "s c")
f.text(400, 398, "The file's clusters are 5, 6 and 9: the directory entry names the first, the FAT links the rest.", "s c")
f.save("ch31-fat.svg")

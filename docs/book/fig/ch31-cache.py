from svg import Fig
f = Fig(800, 400)
f.text(190, 26, "the cache: room for four blocks", "h c")
hdr = [(40, "block"), (120, "contents"), (210, "changed"), (280, "last used")]
for x, t in hdr: f.text(x, 52, t, "s")
rows = [("7", "…", "no", "41", "w"), ("812", "…", "yes", "38", "y"), ("3", "…", "no", "12, the oldest", "r"), ("96", "…", "yes", "40", "y")]
for i, (b, c, d, u, cls) in enumerate(rows):
    y = 62 + i * 34
    f.box(cls, 30, y, 330, 28, 3); f.text(40, y + 19, b, "m"); f.text(120, y + 19, "512 bytes", "s"); f.text(210, y + 19, d, "s"); f.text(280, y + 19, u, "s")
f.box("n", 560, 150, 210, 60); f.text(665, 175, "the disk", "b c"); f.text(665, 194, "slow", "s c")
# cases
def case(y, n, title, lines, col="#2c5d9b"):
    f.num(42, y - 4, n, col); f.text(60, y, title, "b"); f.lines(60, y + 16, lines, "s", 14)
case(226, 1, "read block 7: a hit", ["answered from memory;", "its last-used time is updated"])
case(292, 2, "change block 96", ["changed in memory and marked;", "the disk is not touched yet"])
f.num(302, 222, 3, "#aa2222"); f.text(320, 226, "read block 500: a miss, and the cache is full", "b"); f.lines(320, 242, ["the oldest block, 3, gives up its place", "(written to the disk first if it was changed);", "block 500 is read from the disk into the free place"], "s", 14)
f.num(302, 318, 4, "#3e7b34"); f.text(320, 322, "sync", "b"); f.lines(320, 338, ["every changed block is written, in block order,", "and marked unchanged: 96, then 812"], "s", 14)
f.arrow([(360, 110), (460, 110), (460, 170), (558, 170)], "lng", "ag"); f.text(470, 104, "written at 4, or when evicted", "s")
f.arrow([(558, 196), (440, 196), (440, 144), (362, 144)], "lnr", "ar"); f.text(450, 212, "read at 3", "s")
f.save("ch31-cache.svg")

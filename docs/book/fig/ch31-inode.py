from svg import Fig
f = Fig(800, 360)
f.text(120, 26, "a directory", "h c"); f.text(120, 42, "names to file numbers", "s c")
f.box("y", 30, 52, 180, 112)
for i, (n, ino) in enumerate([("todo.txt", "17"), ("report.txt", "23"), ("old", "9")]):
    y = 62 + i * 32
    f.box("w", 40, y, 110, 26, 2); f.text(48, y + 17, n, "m"); f.box("w", 150, y, 50, 26, 2); f.text(175, y + 17, ino, "m c")
f.text(400, 26, "the file's record (an inode)", "h c"); f.text(400, 42, "everything about file 17 but its name", "s c")
f.box("k", 300, 52, 200, 190)
for i, t in enumerate(["type: regular file", "owner, group, mode", "size: 1300 bytes", "times", "link count: 1"]):
    f.text(312, 74 + i * 18, t, "s")
f.text(312, 172, "blocks:", "s b")
for i, b in enumerate(["812", "813", "96"]):
    f.box("w", 312 + i * 58, 182, 52, 26, 2); f.text(338 + i * 58, 199, b, "m c")
f.text(312, 230, "in order: bytes 0-511, 512-1023, ...", "s")
f.arrow([(200, 75), (298, 75)]); f.text(250, 68, "1", "s c")
f.text(660, 26, "data blocks", "h c"); f.text(660, 42, "the file's bytes", "s c")
for i, (b, cls) in enumerate([("96", "g"), ("97", "n"), ("…", "n"), ("812", "g"), ("813", "g")]):
    y = 56 + i * 38
    f.box(cls, 600, y, 120, 30, 3); f.text(660, y + 20, "block " + b if b != "…" else "…", "m c")
f.arrow([(338, 208), (338, 262), (560, 262), (560, 185), (598, 185)]); 
f.arrow([(396, 208), (396, 276), (574, 276), (574, 223), (598, 223)])
f.arrow([(454, 208), (454, 248), (546, 248), (546, 71), (598, 71)])
f.text(400, 316, "Opening a path walks directories to find a file number (1); reading follows the record to the blocks.", "s c")
f.text(400, 334, "A name is an entry in a directory, not a property of the file: two names may give the same number.", "s c")
f.save("ch31-inode.svg")

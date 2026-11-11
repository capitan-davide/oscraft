from svg import Fig
f = Fig(840, 350)

# Left: mount points, walked inside one kernel.
f.text(205, 24, "Mount points: the kernel walks the path", "h c")
f.box("n", 20, 38, 370, 262)
f.text(36, 58, "a monolithic kernel", "s")
f.box("y", 40, 70, 150, 190, 3)
f.text(115, 90, "root file system", "b c")
for i, t in enumerate(["/", "  usr/", "  bin/", "  tmp/"]):
    f.text(76, 122 + i * 30, t, "m")
f.box("y", 230, 70, 140, 190, 3)
f.text(300, 90, "file system", "b c")
f.text(300, 105, "mounted on /tmp", "s c")
f.text(250, 212, "notes", "m")
f.num(52, 118, 1)
f.num(52, 208, 2)
f.arrow([(52, 128), (52, 198)])
f.arrow([(130, 208), (228, 208)], "lnr", "ar")
f.text(180, 200, "mount point", "s c red")
f.num(312, 208, 3)
f.text(205, 284, "each component is looked up in a directory", "s c")
f.text(205, 322, 'open("/tmp/notes"): three lookups, all in the kernel', "s c")

# Right: prefixes registered by servers, matched as strings.
f.text(630, 24, "Prefixes: a manager matches, a server interprets", "h c")
f.box("n", 440, 38, 200, 150)
f.text(540, 58, "name manager: a table", "b c")
rows = [("/dev/random", "server C", "w"), ("/tmp", "server B", "g"), ("/", "server A", "w")]
for i, (p, s, cls) in enumerate(rows):
    y = 72 + i * 36
    f.box(cls, 452, y, 176, 28, 3)
    f.text(462, y + 18, p, "m")
    f.text(618, y + 18, s, "s e")
f.box("u", 690, 84, 130, 56)
f.text(755, 106, "server B", "b c")
f.text(755, 124, "a file system", "s c")

f.text(512, 222, "the path, as a string", "s")
f.box("g", 468, 232, 60, 30, 3)
f.text(498, 252, "/tmp", "m c")
f.box("y", 528, 232, 84, 30, 3)
f.text(570, 252, "/notes", "m c")
f.text(498, 280, "longest prefix", "s c green")
f.text(580, 280, "the rest", "s c")
f.arrow([(498, 232), (498, 138)], "lng", "ag")
f.arrow([(612, 247), (755, 247), (755, 142)], "ln", "a")
f.text(728, 266, "sent to the server it names", "s c")
f.text(630, 322, 'open("/tmp/notes"): one match, then the server does the rest', "s c")
f.save("ch21-mount-vs-prefix.svg")

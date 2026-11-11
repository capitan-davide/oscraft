from svg import Fig
f = Fig(800, 400)
rows = [("u", "a program", "read(fd, buf, 100) on /notes/todo.txt", "names, files, byte offsets"),
        ("k", "the file system", "which blocks hold bytes 0 to 99 of that file?", "knows the layout on the disk"),
        ("y", "the block cache", "is block 812 in memory already?", "keeps recently used blocks"),
        ("k", "the block device driver", "read block 812", "knows the hardware"),
        ("n", "the disk", "numbered blocks of 512 bytes, nothing else", "")]
asks = ["a file, an offset, a length", "a block number", "a block number", "a command to the hardware"]
y = 16
for i, (c, t, q, s) in enumerate(rows):
    f.box(c, 130, y, 420, 52); f.text(340, y + 22, t, "b c"); f.text(340, y + 40, q, "s c")
    f.text(570, y + 30, s, "s")
    if i < len(rows) - 1:
        f.arrow([(340, y + 52), (340, y + 74)]); f.text(352, y + 67, asks[i], "s")
    y += 76
f.save("ch31-stack.svg")

from svg import Fig
f = Fig(800, 372)
X, W = 40, 250
f.text(X + W/2, 24, "The file on disk", "h c")
rows = [("w", 46, "ELF header", "type, machine, OS/ABI, entry point"),
        ("w", 46, "program headers", "one entry per segment"),
        ("k", 78, "segment 1: code and constants", "read, execute"),
        ("y", 56, "segment 2: initialised data", "read, write"),
        ("n", 46, "sections, symbols, debug data", "for the linker and tools: not loaded")]
y = 36
pos = []
for cls, h, a, b in rows:
    f.box(cls, X, y, W, h, 2); f.text(X + W/2, y + h/2 - 2, a, "c"); f.text(X + W/2, y + h/2 + 13, b, "s c")
    pos.append((y, h)); y += h
# header -> segments
for j in (2, 3):
    yy = pos[j][0] + pos[j][1] / 2
    f.arrow([(X, pos[1][0] + 23), (X - 22, pos[1][0] + 23), (X - 22, yy), (X - 2, yy)], "ln", "a", True)
f.lines(X, y + 26, ["Each program header says: these bytes of the file,", "at this address, this many bytes of memory,", "with these permissions."], "s", 14)
MX, MW = 500, 260
f.text(MX + MW/2, 24, "The program in memory", "h c")
f.text(MX + MW + 8, 52, "low", "s")
mem = [("k", 78, "code and constants", ""),
       ("y", 56, "data", "pages: read, write"),
       ("w", 50, ".bss", "zero-filled; not in the file")]
my = 40
mpos = []
for cls, h, a, b in mem:
    f.box(cls, MX, my, MW, h, 2); f.text(MX + MW/2, my + (h/2 - 2 if b else 24), a, "c"); f.text(MX + MW/2, my + (h/2 + 13 if b else 40), b or "pages: read, execute", "s c")
    mpos.append((my, h)); my += h + (14 if cls == "k" else 0)
f.arrow([(X + W, pos[2][0] + 39), (MX - 3, mpos[0][0] + 39)], "lnk", "ak")
f.text(425, pos[2][0] + 6, "file size = memory size", "s c")
f.arrow([(X + W, pos[3][0] + 28), (MX - 3, mpos[1][0] + 28)], "lnk", "ak")
f.text(395, mpos[1][0] + 72, "memory size > file size:", "s c"); f.text(395, mpos[1][0] + 86, "the rest is zeros", "s c")
# brace for segment 2 in memory
bx = MX + MW + 8
f.line([(bx, mpos[1][0]), (bx + 8, mpos[1][0]), (bx + 8, mpos[2][0] + mpos[2][1]), (bx, mpos[2][0] + mpos[2][1])])
f.text(bx - 4, mpos[2][0] + mpos[2][1] + 18, "segment 2", "s e")
f.box("g", MX + 16, mpos[0][0] + 54, 14, 14, 2)
f.text(MX + 36, mpos[0][0] + 66, "entry point: the first instruction to run", "s")
f.save("ch19-elf-file.svg")

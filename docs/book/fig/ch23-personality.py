from svg import Fig
f = Fig(800, 360)
# ELF header bytes
f.text(20, 30, "the program file's first 16 bytes (e_ident)", "b")
cells = ["7f", "'E'", "'L'", "'F'", "class", "data", "vers.", "OS ABI", "", "", "", "", "", "", "", ""]
x = 20
for i, c in enumerate(cells):
    w = 62 if i == 7 else (44 if i < 8 else 14)
    f.box("y" if i == 7 else "w", x, 40, w, 30, 2)
    if c: f.text(x + w / 2, 60, c, "ms c")
    if i < 8: f.text(x + w / 2, 84, str(i), "s c")
    if i == 7: ox = x + w / 2
    x += w
f.text(x + 10, 60, "bytes 8 to 15: padding", "s")
# decision
f.arrow([(ox, 90), (ox, 118)])
f.box("k", 250, 120, 190, 44, 3); f.text(345, 139, "the loader:", "s c"); f.text(345, 155, "Personality::from_osabi()", "ms c")
f.arrow([(440, 132), (520, 112)], "lng", "ag"); f.box("g", 522, 96, 258, 30, 3); f.text(651, 116, "9, FreeBSD's brand: Native", "c")
f.arrow([(440, 152), (520, 172)], "lnr", "ar"); f.box("r", 522, 158, 258, 30, 3); f.text(651, 178, "any other value: Qnx", "c")
# the process
f.arrow([(345, 164), (345, 198)])
f.box("k", 20, 200, 420, 60); f.text(230, 222, "the process object", "h c"); f.box("y", 150, 230, 160, 22, 3); f.text(230, 246, "personality", "ms c")
f.text(455, 222, "fork(): the child takes its parent's", "s"); f.text(455, 240, "exec: set again from the new file", "s")
# consulted
f.text(20, 290, "asked wherever the two kinds of program differ:", "b")
uses = ["how the program starts", "error numbers of calls", "signals", "process 1's replies", "the message info"]
x = 20
for u in uses:
    f.box("wk", x, 302, 146, 36, 3); f.text(x + 73, 325, u, "s c"); x += 153.5
f.save("ch23-personality.svg")

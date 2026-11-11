from svg import Fig
f = Fig(800, 330)
# the kernel
f.box("k", 250, 20, 300, 290); f.text(400, 42, "the kernel and process 1", "h c")
f.box("wk", 266, 56, 150, 236, 3); f.text(341, 76, "inside: FreeBSD's", "b c")
rows = [(112, "ETIMEDOUT = 60"), (160, "SIGCHLD = 20"), (208, "struct stat: 224 bytes"), (256, "struct termios: 44 bytes")]
for y, t in rows: f.text(341, y + 4, t, "s c")
f.box("y", 432, 56, 104, 236, 3); f.text(484, 76, "conversion", "b c")
for y, t in [(112, "errno_out()"), (160, "signo_out()"), (208, "stat_out()"), (256, "termios_out()")]: f.text(484, y + 4, t, "ms c")
# native process
f.box("g", 20, 56, 170, 236); f.text(105, 78, "a native process", "h c"); f.text(105, 95, "a FreeBSD program", "s c")
# QNX process
f.box("r", 610, 56, 170, 236); f.text(695, 78, "a QNX process", "h c"); f.text(695, 95, "optional", "s c")
for y, t in rows:
    f.arrow([(264, y), (192, y)], "lng", "ag")
    f.arrow([(418, y), (430, y)], "ln", "a")
    f.arrow([(538, y), (608, y)], "lnr", "ar")
for y, t in [(112, "60"), (160, "20"), (208, "224 bytes"), (256, "44 bytes")]: f.text(105, y + 4, t, "c")
for y, t in [(112, "260"), (160, "18"), (208, "120 bytes"), (256, "76 bytes")]: f.text(695, y + 4, t, "c")
f.text(227, 136, "as it is", "s c"); f.text(574, 136, "converted", "s c")
f.save("ch23-boundary.svg")

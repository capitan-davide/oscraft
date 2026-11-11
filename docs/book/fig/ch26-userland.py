from svg import Fig
f = Fig(800, 360)
f.box("u", 30, 16, 740, 50); f.text(400, 37, "programs", "b c"); f.text(400, 54, "the shell, ls, ps, ifconfig ... and yours", "s c")
f.box("g", 30, 78, 480, 50); f.text(270, 99, "libraries", "b c"); f.text(270, 116, "threads, maths, terminals, line editing ...", "s c")
f.box("g", 30, 140, 480, 50); f.text(270, 161, "the C library", "b c"); f.text(270, 178, "printf(), malloc(), fopen(), getpwnam(); wrappers for open(), read(), fork()", "s c")
f.box("g", 530, 78, 240, 112); f.text(650, 104, "the dynamic loader", "b c"); f.lines(650, 124, ["maps the shared libraries a", "program names, and binds its", "calls to them, at every start"], "s c", 15)
f.box("y", 30, 202, 740, 44); f.text(400, 221, "files the libraries expect", "b c"); f.text(400, 237, "/etc/pwd.db   /usr/share/terminfo   /etc/resolv.conf   /etc/protocols", "ms c")
f.line([(10, 262), (790, 262)], "life")
f.text(780, 256, "the userland, above", "s e"); f.text(780, 278, "the kernel's interface, below", "s e")
f.box("k", 30, 290, 740, 50); f.text(400, 311, "the system-call interface", "b c"); f.text(400, 328, "call numbers, argument layouts, struct stat, errno values, signal numbers: the ABI", "s c")
for x in (150, 330, 510):
    f.arrow([(x, 246), (x, 288)], "ln", "a", True)
f.save("ch26-userland.svg")

from svg import Fig
f = Fig(800, 400)
f.text(210, 24, "on FreeBSD", "h c"); f.text(590, 24, "on OsCraft", "h c")
for x in (40, 420):
    f.box("u", x, 40, 340, 44); f.text(x + 170, 60, "the program", "b c"); f.text(x + 170, 76, 'f = fopen("/tmp/notes", "r");', "ms c")
    f.box("g", x, 100, 340, 62); f.text(x + 170, 120, "FreeBSD's libc", "b c"); f.text(x + 170, 137, "fopen() parses the mode, allocates the FILE,", "s c"); f.text(x + 170, 152, "and calls the function __sys_openat()", "s c")
    f.arrow([(x + 170, 84), (x + 170, 98)]); f.arrow([(x + 170, 162), (x + 170, 186)])
f.box("frame", 28, 32, 744, 138, 6); f.text(400, 182, "the same compiled code on both systems", "s c")
# FreeBSD side
x = 40
f.box("g", x, 188, 340, 62); f.text(x + 170, 208, "libsys: FreeBSD's stubs", "b c"); f.text(x + 170, 225, "__sys_openat: put number 499 in a register,", "s c"); f.text(x + 170, 240, "trap; on failure store errno, return -1", "s c")
f.arrow([(x + 170, 250), (x + 170, 284)]); f.text(x + 178, 271, "one trap", "s")
f.box("n", x, 286, 340, 96); f.text(x + 170, 306, "FreeBSD's kernel", "b c")
f.box("w", x + 14, 316, 150, 54, 3); f.text(x + 89, 338, "the table of calls:", "s c"); f.text(x + 89, 354, "499: sys_openat()", "ms c")
f.box("w", x + 176, 316, 150, 54, 3); f.text(x + 251, 338, "the file systems,", "s c"); f.text(x + 251, 354, "in the kernel", "s c")
f.arrow([(x + 164, 343), (x + 174, 343)])
# OsCraft side
x = 420
f.box("k", x, 188, 340, 62); f.text(x + 170, 208, "libsys: OsCraft's", "b c"); f.text(x + 170, 225, "__sys_openat: ask who owns the path, connect", "s c"); f.text(x + 170, 240, "to that server, send it an open request", "s c")
f.arrow([(x + 170, 250), (x + 170, 284)]); f.text(x + 178, 271, "kernel calls", "s")
f.box("k", x, 286, 160, 96); f.text(x + 80, 306, "OsCraft's kernel", "b c"); f.text(x + 80, 330, "carries the", "s c"); f.text(x + 80, 345, "messages", "s c")
f.box("u", x + 180, 286, 160, 96); f.text(x + 260, 306, "servers", "b c"); f.text(x + 260, 330, "the pathname manager,", "s c"); f.text(x + 260, 345, "then tmpfs,", "s c"); f.text(x + 260, 360, "which owns /tmp", "s c")
f.arrow([(x + 160, 334), (x + 178, 334)], "ln", "a", False, True)
f.save("ch24-layer.svg")

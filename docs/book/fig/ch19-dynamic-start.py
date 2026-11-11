from svg import Fig
f = Fig(800, 440)
P, L, G = 120, 410, 680
for x, cls, t in [(P, "k", "process 1"), (L, "g", "ld-elf.so.1"), (G, "u", "the program")]:
    f.box(cls, x - 80, 34, 160, 30); f.text(x, 54, t, "c"); f.line([(x, 64), (x, 400)], "life")
f.line([(L - 80, 24), (G + 80, 24)], "ln"); f.text((L + G) / 2, 18, "code running in the new process", "s c")
def step(x, y, h, n, lines, cls, col, w):
    f.box(cls, x - w/2, y, w, h, 3); f.num(x - w/2 + 15, y + h/2, n, col)
    for i, t in enumerate(lines): f.text(x - w/2 + 32, y + (h - 14*len(lines))/2 + 11 + 14*i, t, "s")
step(P, 80, 76, 1, ["maps the program at 4 GiB", "and ld-elf.so.1 at 16 GiB;", "writes the initial stack;", "starts the first thread"], "k", "#2c5d9b", 220)
f.arrow([(P + 110, 140), (L - 127, 162)], "lnk", "ak")
step(L, 150, 62, 2, ["starts at its own entry point:", "relocates itself, reads AT_BASE,", "AT_PHDR and AT_ENTRY from the stack"], "g", "#3e7b34", 250)
step(L, 222, 62, 3, ["open() and mmap() of libc.so.7,", "libsys.so.7 and the other libraries", "the program needs"], "g", "#3e7b34", 250)
f.arrow([(L - 125, 244), (P + 112, 244)], "ln", "a"); f.arrow([(P + 110, 264), (L - 127, 264)], "lng", "ag")
f.box("k", P - 110, 226, 220, 54, 3); f.lines(P, 242, ["answers them like any other", "open() and mmap():", "path manager, memory manager"], "s c", 14)
step(L, 294, 48, 4, ["resolves symbols, applies relocations,", "then jumps to AT_ENTRY"], "g", "#3e7b34", 250)
f.arrow([(L + 125, 326), (G - 112, 340)], "lng", "ag")
step(G, 318, 48, 5, ["_start: the C library", "initialises itself"], "u", "#9a6b2f", 220)
step(G, 376, 30, 6, ["main(argc, argv, envp)"], "u", "#9a6b2f", 220)
f.text(265, 404, "Time runs downwards. Process 1 never looks inside a shared library.", "s c")
f.save("ch19-dynamic-start.svg")

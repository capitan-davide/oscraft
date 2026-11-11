from svg import Fig
f = Fig(800, 350)
# monolithic
f.text(200, 24, "A monolithic kernel: the kernel routes", "h c")
f.box("u", 40, 40, 320, 40); f.text(200, 58, "C library: one stub per call,", "s c"); f.text(200, 72, "all alike but for the number", "s c")
f.arrow([(200, 80), (200, 112)]); f.text(208, 100, "trap, number N", "s")
f.box("n", 40, 114, 320, 216); f.text(113, 134, "the kernel", "b c")
tab = [("2", "fork"), ("3", "read"), ("5", "open"), ("37", "kill"), ("97", "socket"), ("477", "mmap")]
y = 146
for n, name in tab:
    f.box("w", 56, y, 44, 24, 0); f.text(78, y + 16, n, "ms c"); f.box("w", 100, y, 70, 24, 0); f.text(135, y + 16, name, "ms c"); y += 24
f.text(113, y + 18, "... about 600 numbers", "s c")
subs = [(150, "processes"), (198, "file systems"), (246, "signals"), (294, "network, memory")]
for y2, t in subs:
    f.box("w", 220, y2 - 16, 126, 34, 3); f.text(283, y2 + 5, t, "s c")
f.arrow([(170, 158), (218, 150)]); f.arrow([(170, 182), (218, 194)]); f.arrow([(170, 206), (218, 200)])
f.arrow([(170, 230), (218, 244)]); f.arrow([(170, 254), (218, 290)]); f.arrow([(170, 278), (218, 298)])
# microkernel
f.text(600, 24, "A microkernel: the library routes", "h c")
f.box("u", 440, 40, 320, 112); f.text(600, 58, "C library: each call written out", "s c")
f.box("y", 454, 68, 292, 74, 3)
for i, (a, b) in enumerate([("read", "a message on the descriptor"), ("open", "find the server, then a message"), ("fork", "a message to the process manager"), ("kill", "a kernel call")]):
    f.text(466, 86 + 16 * i, a, "ms"); f.text(510, 86 + 16 * i, b, "ms")
f.box("k", 440, 196, 320, 44); f.text(600, 214, "the kernel: a few dozen calls", "b c"); f.text(600, 230, "threads, messages, signals, timers, locks", "s c")
f.arrow([(520, 152), (520, 194)]); f.text(528, 178, "a call for the kernel itself", "s")
f.box("u", 440, 286, 150, 44); f.text(515, 304, "process manager", "s c"); f.text(515, 320, "memory, path names", "s c")
f.box("u", 610, 286, 150, 44); f.text(685, 304, "file systems,", "s c"); f.text(685, 320, "network, drivers", "s c")
f.arrow([(515, 240), (515, 284)], "lnk", "ak"); f.arrow([(685, 240), (685, 284)], "lnk", "ak"); f.text(600, 266, "messages", "s c")
f.save("ch24-table.svg")

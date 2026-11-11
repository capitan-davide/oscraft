from svg import Fig
f = Fig(800, 420)
f.box("g", 20, 16, 760, 40); f.text(400, 34, "programs", "b c"); f.text(400, 49, "every open file or socket is a connection to one server", "s c")
row1 = [("pipe", 30, 90, None), ("pty", 130, 90, None), ("tmpfs", 230, 90, None), ("syslogd", 330, 90, None), ("random", 430, 90, None), ("diskd", 530, 100, "file systems"), ("netd", 650, 120, "sockets, drivers")]
centers = {}
for n, x, w, sub in row1:
    f.box("k", x, 96, w, 34); centers[n] = x + w / 2
    if sub: f.text(x + w / 2, 111, n, "c"); f.text(x + w / 2, 124, sub, "s c")
    else: f.text(x + w / 2, 117, n, "c")
    f.arrow([(x + w / 2, 56), (x + w / 2, 94)])
f.box("k", 590, 196, 90, 34); f.text(635, 211, "pcid", "c"); f.text(635, 224, "the PCI bus", "s c")
f.box("k", 700, 196, 80, 34); f.text(740, 211, "usbd", "c"); f.text(740, 224, "USB", "s c")
f.arrow([(690, 130), (648, 194)], "lnk", "ak")
f.arrow([(730, 130), (740, 194)], "lnk", "ak")
f.arrow([(698, 213), (682, 213)], "lnk", "ak")
f.box("k", 20, 300, 760, 46); f.text(400, 319, "process 1: processes, memory, path names", "b c"); f.text(400, 335, "every program and every server sends to it; it sends to nobody", "s c")
f.box("k", 20, 366, 760, 36); f.text(400, 388, "microkernel: carries the messages, schedules the threads", "c")
for n in ["pipe", "pty", "tmpfs", "syslogd", "random", "diskd"]:
    f.arrow([(centers[n], 130), (centers[n], 298)], "ln", "a", True)
f.arrow([(635, 230), (635, 298)], "ln", "a", True); f.arrow([(740, 230), (740, 298)], "ln", "a", True)
f.text(85, 262, "for memory, path names", "s"); f.text(85, 276, "and child processes", "s")
f.text(30, 80, "requests", "s")
f.save("ch02-servers.svg")

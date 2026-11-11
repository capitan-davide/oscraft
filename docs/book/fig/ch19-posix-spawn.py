from svg import Fig
f = Fig(800, 460)
A, P, C = 120, 400, 680
for x, cls, t in [(A, "u", "parent"), (P, "k", "process 1"), (C, "u", "child")]:
    f.box(cls, x - 80, 30, 160, 30); f.text(x, 50, t, "c"); f.line([(x, 60), (x, 430)], "life")
def blk(x, y, h, lines, cls, w=200):
    f.box(cls, x - w/2, y, w, h, 3)
    for i, t in enumerate(lines): f.text(x, y + (h - 14*len(lines))/2 + 11 + 14*i, t, "s c")
blk(A, 72, 48, ["posix_spawn() in libc:", "rfork(RFSPAWN) gives EINVAL,", "so it calls vfork()"], "wu")
f.arrow([(A, 136), (P - 2, 136)], "lnk", "ak"); f.num(250, 122, 1, "#2c5d9b"); f.text(264, 126, "_PROC_FORK", "ms")
blk(A, 144, 150, ["REPLY-blocked"], "y", 110)
blk(P, 144, 34, ["copies the process"], "k")
f.arrow([(P + 100, 168), (C - 2, 186)], "lnk", "ak")
blk(C, 188, 48, ["tells the servers about its", "copied descriptors (Chapter 18)"], "u")
f.arrow([(C - 100, 228), (P + 2, 248)], "ln", "a", True); f.text(540, 254, "pulse: ready", "s c")
f.arrow([(P, 294), (A + 57, 294)], "lng", "ag"); f.num(250, 280, 2, "#3e7b34"); f.text(264, 284, "reply: the child's ID", "s")
blk(A, 302, 48, ["posix_spawn() returns", "the child's ID"], "g", 180)
blk(C, 262, 60, ["fork() returns 0: libc applies the", "attributes and file actions with", "dup2(), close(), open(), sigprocmask()"], "wu", 230)
f.arrow([(C, 340), (P + 2, 340)], "lnk", "ak"); f.num(530, 326, 3, "#2c5d9b"); f.text(544, 330, "_PROC_SPAWN, exec flag", "ms")
blk(P, 348, 34, ["replaces the child's image"], "k")
f.arrow([(P + 100, 372), (C - 2, 392)], "lng", "ag")
blk(C, 394, 30, ["the new program runs"], "g")
f.text(120, 384, "The parent has its answer", "s c"); f.text(120, 398, "before the child's exec.", "s c")
f.save("ch19-posix-spawn.svg")

from svg import Fig
f = Fig(800, 400)
P, K, O, C = 90, 330, 560, 720
for n, c, x, w in [("parent (sh)", "u", P, 140), ("sysmgr thread", "k", K, 160), ("another client", "u", O, 130), ("child (ls)", "u", C, 110)]:
    f.box(c, x - w / 2, 14, w, 30); f.text(x, 34, n, "c"); f.line([(x, 44), (x, 384)], "life")
f.arrow([(P, 70), (K - 2, 70)], "lnk", "ak"); f.num(P + 16, 58, 1, "#2c5d9b"); f.text(P + 30, 62, "wait request", "s")
f.box("y", P - 45, 76, 90, 250, 3); f.text(P, 196, "REPLY-", "s c"); f.text(P, 209, "blocked", "s c")
f.box("wk", K - 100, 80, 200, 46, 3); f.lines(K, 98, ["no child has ended: keep the", "receive ID in a list, reply later"], "s c", 14); f.num(K - 114, 103, 2)
f.box("g", C - 40, 60, 80, 180, 3); f.text(C, 150, "runs", "s c")
f.arrow([(O, 160), (K + 2, 160)], "lnk", "ak"); f.num(O - 110, 148, 3, "#2c5d9b"); f.text(O - 96, 152, "mmap request", "s")
f.arrow([(K, 190), (O - 2, 190)], "lng", "ag"); f.text(450, 184, "reply", "s c")
f.text(K - 10, 178, "served at once", "s e")
f.arrow([(C, 250), (K + 2, 250)], "ln", "a", True); f.num(C - 60, 238, 4); f.text(C - 74, 242, "exits: the kernel sends a pulse", "s e")
f.box("wk", K - 100, 260, 200, 46, 3); f.lines(K, 278, ["look through the held waits:", "this one is now satisfied"], "s c", 14); f.num(K - 114, 283, 5)
f.arrow([(K - 100, 326), (P + 47, 326)], "lng", "ag"); f.num(190, 314, 6, "#3e7b34"); f.text(204, 318, "reply: the status", "s")
f.box("g", P - 45, 334, 90, 36, 3); f.text(P, 356, "wait() returns", "s c")
f.save("ch18-held.svg")

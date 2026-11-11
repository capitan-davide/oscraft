from svg import Fig
f = Fig(800, 440)
P, K, C, S = 80, 310, 530, 730
for n, c, x, w in [("parent", "u", P, 130), ("process 1 (sysmgr)", "k", K, 170), ("child", "u", C, 130), ("tmpfs", "u", S, 100)]:
    f.box(c, x - w / 2, 14, w, 30); f.text(x, 34, n, "c"); f.line([(x, 44), (x, 424)], "life")
f.arrow([(P, 70), (K - 2, 70)], "lnk", "ak"); f.num(P + 16, 58, 1, "#2c5d9b"); f.text(P + 30, 62, "fork request", "s")
f.box("y", P - 45, 76, 90, 290, 3); f.text(P, 200, "REPLY-", "s c"); f.text(P, 213, "blocked", "s c"); f.text(P, 236, "its registers", "s c"); f.text(P, 249, "are saved", "s c")
f.box("wk", K - 110, 80, 220, 104, 3); f.num(K - 124, 132, 2)
f.lines(K, 98, ["a new process: copies of the", "descriptors, credentials, signals", "copy the memory", "note: answer the parent later", "a thread from the parent's registers,", "its result changed"], "s c", 14)
f.arrow([(K + 110, 170), (C - 2, 170)], "lnk", "ak", True); f.text(440, 164, "ready to run", "s c")
f.box("wu", C - 80, 184, 160, 46, 3); f.lines(C, 202, ["still in the kernel:", "one message per descriptor"], "s c", 14); f.num(C - 94, 207, 3)
f.arrow([(C + 80, 222), (S - 2, 222)], "ln", "a"); f.text(670, 200, "\"a copy of the", "s c"); f.text(670, 213, "parent's descriptor 3\"", "s c")
f.arrow([(S, 254), (C + 2, 254)], "lng", "ag"); f.text(670, 268, "shares the open file", "s c")
f.arrow([(C, 286), (K + 2, 286)], "ln", "a", True); f.num(C - 16, 274, 4); f.text(C - 30, 278, "pulse: start-up done", "s e")
f.box("wk", K - 80, 296, 160, 30, 3); f.text(K, 315, "answer the held request", "s c")
f.arrow([(K - 80, 366), (P + 47, 366)], "lng", "ag"); f.num(170, 354, 5, "#3e7b34"); f.text(184, 358, "reply: the child's ID", "s")
f.box("g", P - 60, 376, 120, 40, 3); f.lines(P, 392, ["fork() returns", "the child's ID"], "s c", 14)
f.box("g", C - 80, 300, 160, 70, 3); f.lines(C, 318, ["returns to user mode, from", "the same call as the parent:", "its result has a mark,", "so fork() returns 0"], "s c", 14)
f.save("ch18-fork.svg")

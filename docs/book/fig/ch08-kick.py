from svg import Fig
f = Fig(800, 390)
xs = [40, 230, 420, 610]
info = [("CPU 0", "runs priority 60", "k"), ("CPU 1", "runs priority 40", "n"), ("CPU 2", "runs priority 10", "g"), ("CPU 3", "runs priority 30", "n")]
for x, (n, r, c) in zip(xs, info):
    f.box(c, x, 40, 150, 46); f.text(x + 75, 60, n, "b c"); f.text(x + 75, 77, r, "s c")
    f.line([(x + 75, 86), (x + 75, 330)], "life")
f.text(115, 28, "the waker", "s c"); f.text(495, 28, "the lowest priority below 50", "s c green")
# steps on CPU 0
f.box("wk", 40, 104, 150, 50, 3); f.num(52, 118, 1, "#2c5d9b"); f.text(66, 122, "wake(T): T, priority", "s"); f.text(66, 136, "50, goes on the", "s"); f.text(66, 149, "ready queue", "s")
f.box("wk", 40, 166, 150, 64, 3); f.num(52, 180, 2, "#2c5d9b"); f.text(66, 184, "kick_for(T): reads", "s"); f.text(66, 198, "each CPU's", "s"); f.text(66, 212, "running_prio,", "s"); f.text(66, 225, "chooses CPU 2", "s")
f.box("wk", 40, 242, 150, 50, 3); f.num(52, 256, 3, "#2c5d9b"); f.text(66, 260, "ipi::send(2, RESCHED)", "s"); f.text(66, 274, "through startup's", "s"); f.text(66, 287, "send_ipi callout", "s")
f.arrow([(190, 267), (418, 267)], "lnk", "ak"); f.text(305, 259, "interprocessor interrupt", "s c blue")
f.box("g", 420, 244, 150, 46, 3); f.num(432, 258, 4, "#3e7b34"); f.text(446, 262, "IPI handler: sets", "s"); f.text(446, 276, "need_resched", "s")
f.box("g", 420, 298, 150, 60, 3); f.num(432, 312, 5, "#3e7b34"); f.text(446, 316, "leaving the interrupt:", "s"); f.text(446, 330, "preempt(), schedule()", "s"); f.text(446, 344, "switches to T", "s b")
f.text(495, 120, "its thread (priority 10)", "s c"); f.text(495, 134, "runs on until the", "s c"); f.text(495, 148, "interrupt arrives", "s c")
f.text(305, 120, "not asked:", "s c"); f.text(305, 134, "40 is not the lowest", "s c")
f.text(685, 120, "not asked:", "s c"); f.text(685, 134, "30 is not the lowest", "s c")
f.text(400, 380, "An idle CPU is chosen before any busy one. If the waker itself runs below T and no CPU is idle, it switches by itself and sends nothing.", "s c")
f.save("ch08-kick.svg")

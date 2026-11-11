from svg import Fig
f = Fig(800, 330)
f.box("u", 20, 40, 190, 76); f.text(115, 62, "generated", "b c"); f.lines(115, 80, ["kill(), a fault, a timer,", "a child ending, the terminal"], "s c", 14)
f.box("y", 290, 40, 200, 76); f.text(390, 62, "pending", "b c"); f.lines(390, 80, ["recorded for the process", "or for one thread"], "s c", 14)
f.box("k", 570, 40, 210, 76); f.text(675, 62, "delivered", "b c"); f.lines(675, 80, ["to a thread that", "does not block it"], "s c", 14)
f.arrow([(210, 78), (288, 78)]); f.arrow([(490, 78), (568, 78)])
# blocked loop
f.arrow([(430, 116), (430, 150), (350, 150), (350, 118)], "ln", "a", True)
f.text(390, 166, "blocked by every thread that", "s c"); f.text(390, 180, "could take it: it waits here", "s c")
# actions
f.text(688, 140, "the action decides", "s")
f.arrow([(675, 116), (675, 150)])
A = [("n", "ignore", "the signal is discarded"), ("r", "default", "for most signals: the process ends"), ("g", "handler", "a function of the program runs")]
for i, (c, a, b) in enumerate(A):
    y = 152 + i * 56
    f.box(c, 560, y, 220, 44); f.text(670, y + 18, a, "b c"); f.text(670, y + 34, b, "s c")
f.text(20, 250, "A signal whose action is to ignore it is normally dropped when it is generated.", "s")
f.text(20, 266, "A blocked signal is kept even then: the action may have changed by the time", "s")
f.text(20, 282, "it is unblocked.", "s")
f.save("ch14-life.svg")

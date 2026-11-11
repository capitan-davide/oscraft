from svg import Fig
f = Fig(800, 360)
# devices
devs = [("network device", 60), ("serial port", 130), ("disk controller", 200)]
f.text(90, 36, "Devices", "h c")
for name, y in devs:
    f.box("n", 20, y, 140, 40); f.text(90, y + 24, name, "c")
    f.arrow([(160, y + 20), (250, y + 20)])
f.text(205, 52, "interrupt", "s c"); f.text(205, 64, "lines", "s c")
# controller
f.box("n", 250, 44, 300, 212)
f.text(400, 36, "Interrupt controller", "h c")
cols = [("enabled?", 262), ("pending?", 352), ("priority, CPU", 442)]
for t, x in cols:
    f.text(x + 4, 60, t, "s b")
rows = [("yes", "yes", "high, CPU 0", 60), ("yes", "no", "low, CPU 0", 130), ("masked", "yes", "low, CPU 1", 200)]
for en, pe, pr, y in rows:
    f.box("r" if en == "masked" else "g", 262, y + 6, 78, 28, 3); f.text(301, y + 24, en, "s c")
    f.box("y" if pe == "yes" else "w", 352, y + 6, 78, 28, 3); f.text(391, y + 24, pe, "s c")
    f.box("w", 442, y + 6, 96, 28, 3); f.text(490, y + 24, pr, "s c")
f.text(400, 248, "picks the highest-priority pending, enabled line", "s c")
# CPUs
f.text(700, 36, "Processors", "h c")
f.box("k", 640, 60, 120, 60); f.text(700, 86, "CPU 0", "b c"); f.text(700, 102, "handler runs", "s c")
f.box("k", 640, 180, 120, 60); f.text(700, 206, "CPU 1", "b c"); f.text(700, 222, "not interrupted", "s c")
f.arrow([(550, 68), (640, 68)], "lnr", "ar"); f.text(618, 62, "IRQ", "s c red"); f.num(578, 68, 2)
f.arrow([(640, 90), (550, 90)], "lnk", "ak"); f.num(596, 90, 3, "#2c5d9b")
f.arrow([(640, 112), (550, 112)], "lnk", "ak"); f.num(614, 112, 5, "#2c5d9b")
f.num(205, 80, 1); f.num(750, 70, 4)
# steps
f.num(30, 286, 1); f.text(46, 290, "A device raises its line.", "s")
f.num(30, 308, 2); f.text(46, 312, "The controller marks it pending and, if it is enabled, signals a CPU.", "s")
f.num(30, 330, 3, "#2c5d9b"); f.text(46, 334, "The CPU's handler acknowledges: it asks which interrupt this is.", "s")
f.num(430, 286, 4); f.text(446, 290, "The handler for that interrupt runs.", "s")
f.num(430, 308, 5, "#2c5d9b"); f.text(446, 312, "End of interrupt: the controller may", "s"); f.text(446, 325, "signal the next one. The third line stays", "s"); f.text(446, 338, "pending until software unmasks it.", "s")
f.save("ch15-controller.svg")

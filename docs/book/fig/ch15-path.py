from svg import Fig
f = Fig(800, 470)
# hardware
f.box("n", 20, 20, 170, 44); f.text(105, 39, "device", "b c"); f.text(105, 54, "raises its interrupt line", "s c")
f.box("n", 20, 92, 170, 44); f.text(105, 111, "GICv2", "b c"); f.text(105, 126, "signals a CPU", "s c")
f.arrow([(105, 64), (105, 92)])
# kernel column
KX, KW = 250, 270
def kbox(y, h, title, subs):
    f.box("k", KX, y, KW, h); f.text(KX + KW / 2, y + 19, title, "b c")
    for i, s_ in enumerate(subs): f.text(KX + KW / 2, y + 34 + i * 13, s_, "s c")
kbox(92, 44, "exception vector: IRQ", ["the interrupted thread's registers are saved"])
f.arrow([(190, 114), (KX, 114)], "lnr", "ar"); f.text(212, 108, "IRQ", "s c red")
kbox(160, 70, "interrupt::handle()", ["for each controller: run its id fragment", "(-1: not mine, try the next)", "vector = base + cpu × stride + number"])
kbox(254, 44, "the kernel's handler for the vector", ["one function per vector"])
kbox(322, 44, "the controller's eoi fragment", ["end of interrupt"])
kbox(390, 44, "sched::preempt()", ["switch if a handler made a thread ready"])
for y0, y1 in [(136, 160), (230, 254), (298, 322), (366, 390)]:
    f.arrow([(KX + KW / 2, y0), (KX + KW / 2, y1)])
f.num(KX - 14, 114, 1); f.num(KX - 14, 195, 2); f.num(KX - 14, 276, 3); f.num(KX - 14, 344, 4); f.num(KX - 14, 412, 5)
# system page
f.box("y", 20, 176, 170, 84)
f.text(105, 196, "system page", "b c"); f.lines(105, 212, ["per controller: id and eoi", "code fragments, mask and", "unmask callouts"], "s c", 13)
f.box("wk", 20, 290, 170, 76, 3)
f.text(105, 310, "at boot", "b c"); f.lines(105, 326, ["fragments copied into one", "page of kernel code,", "each ended with a return"], "s c", 13)
f.arrow([(105, 260), (105, 290)])
f.arrow([(190, 328), (222, 328), (222, 214), (KX, 214)], "lnk", "ak", True)
f.arrow([(222, 328), (222, 358), (KX, 358)], "lnk", "ak", True)
# handlers
HX = 580
def hbox(y, h, title, subs, cls="wk"):
    f.box(cls, HX, y, 200, h, 3); f.text(HX + 100, y + 18, title, "m c")
    for i, s_ in enumerate(subs): f.text(HX + 100, y + 33 + i * 13, s_, "s c")
hbox(176, 40, "clock::tick", ["each CPU's timer vector"])
hbox(226, 40, "ipi::handler", ["each CPU's IPI vector"])
hbox(276, 68, "ker::interrupt::irq", ["a vector a driver attached to:", "mask it at the controller,", "wake the driver's thread"])
f.line([(KX + KW, 276), (550, 276)], "lnk")
f.line([(550, 196), (550, 310)], "lnk")
for y in (196, 246, 310):
    f.arrow([(550, y), (HX, y)], "lnk", "ak")
f.box("u", HX, 384, 200, 66); f.text(HX + 100, 404, "the driver's interrupt thread", "b c"); f.lines(HX + 100, 420, ["in its own process:", "InterruptWait() returns"], "s c", 13)
f.arrow([(HX + 100, 344), (HX + 100, 384)], "lng", "ag")
f.text(20, 400, "Steps 2 to 4 run with interrupts masked,", "s"); f.text(20, 414, "on the kernel stack of the thread that", "s"); f.text(20, 428, "was interrupted.", "s")
f.save("ch15-path.svg")

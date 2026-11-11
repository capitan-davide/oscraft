from svg import Fig
f = Fig(800, 350)
def panel(x, title):
    f.text(x + 125, 24, title, "h c")
P = [15, 275, 535]
# 1 from outside
panel(P[0], "From outside the machine")
x = P[0]
f.box("u", x + 55, 40, 140, 32); f.text(x + 125, 60, "debugger", "c")
f.arrow([(x + 125, 72), (x + 125, 98)], "ln", "a", start=True)
f.box("n", x + 20, 100, 210, 32); f.text(x + 125, 120, "emulator, or a hardware probe", "c")
f.arrow([(x + 125, 132), (x + 125, 158)], "ln", "a", start=True)
f.box("k", x + 20, 160, 210, 60); f.text(x + 125, 182, "the whole machine, stopped:", "c"); f.text(x + 125, 198, "all CPUs, all memory", "c"); f.text(x + 125, 213, "the kernel does not take part", "s c")
f.lines(x + 125, 250, ["Works however broken the", "kernel is, from its first instruction.", "Needs an emulator or a probe."], "s c", 14)
# 2 from inside
panel(P[1], "From inside the kernel")
x = P[1]
f.box("u", x + 55, 40, 140, 32); f.text(x + 125, 60, "debugger", "c")
f.arrow([(x + 125, 72), (x + 125, 158)], "ln", "a", start=True); f.text(x + 133, 112, "serial line", "s"); f.text(x + 133, 126, "or network", "s")
f.box("k", x + 20, 160, 210, 60); f.text(x + 125, 180, "the kernel", "c")
f.box("wk", x + 50, 188, 150, 24); f.text(x + 125, 204, "a debug stub inside it", "s c")
f.lines(x + 125, 250, ["Works on real hardware in the field.", "Needs the kernel healthy enough to", "run the stub and its driver."], "s c", 14)
# 3 afterwards
panel(P[2], "After the crash")
x = P[2]
f.box("k", x + 20, 40, 210, 32); f.text(x + 125, 60, "the kernel crashes", "c")
f.arrow([(x + 125, 72), (x + 125, 98)], "lnr", "ar")
f.box("r", x + 20, 100, 210, 32); f.text(x + 125, 120, "a dumper copies memory out", "c")
f.arrow([(x + 125, 132), (x + 125, 158)])
f.box("y", x + 20, 160, 100, 60); f.text(x + 70, 186, "crash dump", "c"); f.text(x + 70, 202, "a file", "s c")
f.arrow([(x + 120, 190), (x + 138, 190)])
f.box("u", x + 140, 160, 90, 60); f.text(x + 185, 186, "debugger", "c"); f.text(x + 185, 202, "any time later", "s c")
f.lines(x + 125, 250, ["Nothing is running any more: only", "reading. The one way to study a", "crash that cannot be repeated."], "s c", 14)
f.line([(268, 14), (268, 290)], "life"); f.line([(528, 14), (528, 290)], "life")
f.text(400, 326, "A process is debugged with the kernel's help. The kernel has nobody below it to ask.", "s c")
f.save("ch36-kernel-debug.svg")

from svg import Fig
f = Fig(800, 380)
def strip(y, title, mid_cls, mid, note):
    f.text(150, y + 19, title, "e b")
    cells = [("w", "add x0, x0, x1"), (mid_cls, mid), ("w", "mov x2, x0"), ("w", "ret")]
    for i, (cls, t) in enumerate(cells):
        f.box(cls, 170 + i * 130, y, 130, 30, 0); f.text(235 + i * 130, y + 19, t, "m c")
    f.text(700, y + 19, note, "s")
strip(40, "the program", "w", "bl compute", "")
strip(110, "breakpoint set", "r", "trap", "")
f.num(345, 90, 1, "#2c5d9b"); f.text(380, 94, "the debugger asks; the system saves the instruction and writes a trap", "s")
f.arrow([(365, 72), (365, 108)], "lnk", "ak")
# hit
f.num(345, 166, 2, "#aa2222"); f.text(380, 170, "a thread gets there: the trap enters the kernel", "s")
f.arrow([(365, 142), (365, 188)], "lnr", "ar")
f.box("k", 170, 190, 250, 50); f.text(295, 211, "kernel: stops every thread of", "c"); f.text(295, 227, "the process, records why and who", "c")
f.arrow([(420, 215), (478, 215)], "ln", "a", True); f.num(449, 200, 3); 
f.box("u", 480, 190, 290, 50); f.text(625, 211, "debugger: is told, reads registers", "c"); f.text(625, 227, "and memory, shows the source line", "c")
# continue
f.text(150, 289, "to go on", "e b")
steps = [("put the instruction", "back"), ("run that one", "instruction (step)"), ("write the trap", "again"), ("let the process", "run")]
for i, (a, b) in enumerate(steps):
    x = 170 + i * 152
    f.box("u", x, 266, 136, 44); f.text(x + 68, 284, a, "s c"); f.text(x + 68, 298, b, "s c")
    if i:
        f.arrow([(x - 16, 288), (x - 1, 288)])
f.num(80, 285, 4)
f.text(400, 346, "While the trap is in place, a debugger that reads the program's code is shown the saved instruction,", "s c")
f.text(400, 362, "so that it never mistakes its own breakpoint for part of the program.", "s c")
f.save("ch36-breakpoint.svg")

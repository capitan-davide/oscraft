from svg import Fig
f = Fig(800, 412)
f.text(20, 24, "What the thread executes", "h")
Y = 76
f.box("g", 20, Y, 220, 40); f.text(130, Y + 25, "the program's code", "c")
f.box("k", 240, Y, 60, 40); f.text(270, Y + 18, "kernel", "s c"); f.text(270, Y + 32, "saves", "s c")
f.box("u", 300, Y, 200, 40); f.text(400, Y + 25, "the handler", "c")
f.box("k", 500, Y, 60, 40); f.text(530, Y + 18, "kernel", "s c"); f.text(530, Y + 32, "restores", "s c")
f.box("g", 560, Y, 220, 40); f.text(670, Y + 25, "the program's code", "c")
f.arrow([(20, Y + 58), (780, Y + 58)]); f.text(780, Y + 74, "time", "s e")
f.line([(240, Y - 14), (240, Y)], "lnr"); f.text(240, Y - 18, "a signal is delivered here, between two instructions", "s c red")
f.text(670, Y - 6, "continues where it was interrupted", "s c")
# stacks
f.text(20, 172, "The thread's stack (it grows downwards)", "h")
def stack(x, title, parts):
    f.text(x + 100, 196, title, "s c")
    y = 204
    for cls, h, lines in parts:
        f.box(cls, x, y, 200, h, 0)
        for i, t in enumerate(lines):
            f.text(x + 100, y + h / 2 + 4 - (len(lines) - 1) * 7 + i * 14, t, "s c")
        y += h
    return y
stack(40, "before", [("g", 60, ["the program's frames"]), ("w", 110, ["free"])])
stack(300, "while the handler runs", [("g", 60, ["the program's frames"]), ("y", 60, ["saved context: registers,", "program counter, signal mask"]), ("u", 30, ["the handler's frames"]), ("w", 20, [""])])
stack(560, "after", [("g", 60, ["the program's frames"]), ("w", 110, ["free"])])
f.arrow([(245, 289), (295, 289)]); f.arrow([(505, 289), (555, 289)])
f.text(400, 398, "The interrupted code never called the handler. The kernel saves its state on the thread's stack, and puts it back afterwards.", "s c")
f.save("ch14-handler.svg")

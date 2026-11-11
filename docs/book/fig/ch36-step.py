from svg import Fig
f = Fig(800, 300)
cols = [("u", "debugger", ["\"run, one step\""]),
        ("k", "kernel", ["sets the step bit in the", "thread's saved flags", "(SPSR.SS), wakes it"]),
        ("k", "the thread's return", ["turns stepping on for", "this CPU (MDSCR.SS),", "returns to user mode"]),
        ("u", "user mode", ["exactly one instruction", "of the program runs"]),
        ("k", "step exception", ["stepping off, bit cleared;", "the process stops; the", "debugger is told"])]
W, G, X0, Y = 138, 16, 16, 50
for i, (cls, title, lines) in enumerate(cols):
    x = X0 + i * (W + G)
    f.box(cls, x, Y, W, 96); f.text(x + W // 2, Y + 22, title, "b c")
    f.lines(x + W // 2, Y + 46, lines, "s c", 14)
    f.num(x + 12, Y - 12, i + 1, "#2c5d9b" if cls == "k" else "#9a6b2f")
    if i:
        f.arrow([(x - G, Y + 48), (x - 1, Y + 48)])
x3 = X0 + 2 * (W + G) + W // 2
f.box("r", 250, 196, 300, 50); f.text(400, 217, "a context switch on this CPU", "c"); f.text(400, 234, "turns stepping off before the next thread runs", "s c")
f.arrow([(x3, Y + 96), (x3, 194)], "lnr", "ar", True)
f.text(400, 274, "The step bit travels with the one thread's saved state; the CPU's switch is on only for that thread's return.", "s c")
f.text(400, 290, "No other thread, and no kernel code, is ever stepped.", "s c")
f.save("ch36-step.svg")

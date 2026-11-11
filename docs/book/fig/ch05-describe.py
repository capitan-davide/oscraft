from svg import Fig
f = Fig(800, 305)
def col(x, title, sub):
    f.text(x + 120, 24, title, "b c"); f.text(x + 120, 40, sub, "s c")
W = 240
# 1 constants
x = 15
col(x, "1. Built for one machine", "addresses are constants in the kernel")
f.box("k", x, 56, W, 150); f.text(x + W / 2, 76, "kernel", "b c")
f.box("wk", x + 14, 88, W - 28, 44, 3); f.text(x + W / 2, 106, "UART = 0x09000000", "ms c"); f.text(x + W / 2, 121, "TIMER_IRQ = 27  ...", "ms c")
f.box("wk", x + 14, 142, W - 28, 50, 3); f.text(x + W / 2, 163, "drivers for this machine's", "s c"); f.text(x + W / 2, 177, "controller, timer, UART", "s c")
f.text(x + W / 2, 228, "Simple. Another machine needs", "s c"); f.text(x + W / 2, 242, "another kernel.", "s c")
# 2 description + drivers
x = 280
col(x, "2. A description the kernel reads", "a device tree or firmware tables")
f.box("y", x, 56, W, 34); f.text(x + W / 2, 77, "device tree: what is where", "s c")
f.arrow([(x + W / 2, 90), (x + W / 2, 108)], "ln", "a")
f.box("k", x, 110, W, 96); f.text(x + W / 2, 128, "kernel", "b c")
f.box("wk", x + 14, 138, W - 28, 56, 3); f.text(x + W / 2, 156, "a parser, and a driver for", "s c"); f.text(x + W / 2, 170, "every controller, timer and", "s c"); f.text(x + W / 2, 184, "UART it may meet", "s c")
f.text(x + W / 2, 228, "One kernel for many machines,", "s c"); f.text(x + W / 2, 242, "and all their drivers inside it.", "s c")
# 3 a program before the kernel
x = 545
col(x, "3. A program before the kernel", "written for the machine")
f.box("k", x, 56, W, 40); f.text(x + W / 2, 73, "startup program", "b c"); f.text(x + W / 2, 88, "knows this machine", "s c")
f.arrow([(x + W / 2, 96), (x + W / 2, 112)], "ln", "a")
f.box("y", x, 114, W, 46); f.text(x + W / 2, 132, "a fixed table of facts, and a", "s c"); f.text(x + W / 2, 146, "few small routines for the devices", "s c")
f.arrow([(x + W / 2, 160), (x + W / 2, 176)], "ln", "a")
f.box("k", x, 178, W, 28); f.text(x + W / 2, 196, "kernel: knows the processor only", "s c")
f.text(x + W / 2, 228, "One kernel, no drivers in it.", "s c"); f.text(x + W / 2, 242, "A new machine needs a new startup.", "s c")
f.line([(268, 14), (268, 250)], "life"); f.line([(533, 14), (533, 250)], "life")
f.text(400, 284, "The ways combine: a startup program may itself read a device tree for the facts that vary.", "s c")
f.save("ch05-describe.svg")

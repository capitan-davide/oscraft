from svg import Fig
f = Fig(800, 400)
# user mode
f.text(40, 28, "user mode", "s red")
f.box("u", 40, 36, 720, 46); f.text(400, 56, "a program", "c"); f.text(400, 72, "runs its own instructions, in its own memory", "s c")
f.line([(20, 128), (780, 128)], "lnr", True)
f.text(40, 146, "kernel mode", "s red")
# three ways in
ways = [(150, "a system call", "the program asks"), (330, "a fault", "an instruction fails"), (510, "an interrupt", "a device or a timer calls")]
for i, (x, a, b) in enumerate(ways):
    f.arrow([(x, 82), (x, 160)])
    f.num(x - 16, 104, i + 1)
    f.text(x + 8, 101, a, "b")
    f.text(x + 8, 115, b, "s")
f.box("n", 40, 160, 560, 50)
f.text(320, 180, "the processor", "c")
f.text(320, 196, "keeps the return address and the status, raises the privilege, masks interrupts, jumps to the kernel's entry", "s c")
f.arrow([(320, 210), (320, 236)])
f.box("k", 40, 236, 560, 50)
f.text(320, 256, "the kernel's entry code", "c")
f.text(320, 272, "saves every other register of the interrupted program, on a stack of the kernel's own", "s c")
f.arrow([(320, 286), (320, 312)])
f.box("k", 40, 312, 560, 50)
f.text(320, 332, "the handler", "c")
f.text(320, 348, "does the call, repairs or reports the fault, serves the device; may decide that another thread runs next", "s c")
# the way out
f.box("n", 640, 236, 130, 126)
f.text(705, 276, "return from", "c"); f.text(705, 292, "exception", "c")
f.text(705, 312, "restores the status,", "s c"); f.text(705, 326, "drops the privilege,", "s c"); f.text(705, 340, "jumps back", "s c")
f.arrow([(600, 337), (640, 337)], "lnk", "ak")
f.arrow([(705, 236), (705, 82)])
f.text(697, 176, "the only", "s e"); f.text(697, 190, "way out", "s e")
f.text(400, 386, "grey: done by the processor itself; blue: the kernel's code", "s c")
f.save("ch06-modes.svg")

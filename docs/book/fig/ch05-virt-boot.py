from svg import Fig
f = Fig(800, 400)
# QEMU
f.box("n", 20, 20, 150, 92); f.text(95, 40, "QEMU", "b c")
for i, t in enumerate(["-kernel oscraft.ifs:", "copies the image into", "RAM, writes a device", "tree, starts CPU 0"]): f.text(95, 58 + i * 13, t, "s c")
# RAM strip
f.text(210, 30, "RAM, from 0x40000000", "h")
X, Y, H = 210, 44, 46
parts = [("n", 44, ["jump"]), ("y", 62, ["startup", "header"]), ("k", 96, ["startup"]), ("k", 120, ["kernel"]), ("y", 70, ["boot", "script"]), ("g", 122, ["files: servers,", "libc, sh, tools"])]
x = X
pos = {}
for cls, w, ls in parts:
    f.box(cls, x, Y, w, H, 2)
    if len(ls) == 1: f.text(x + w / 2, Y + 27, ls[0], "s c")
    else: f.text(x + w / 2, Y + 20, ls[0], "s c"); f.text(x + w / 2, Y + 34, ls[1], "s c")
    pos[ls[0]] = (x, w); x += w
f.text(X, Y + H + 16, "0x40080000", "ms")
f.line([(X + 202, Y + H + 6), (X + 202, Y + H + 12), (x, Y + H + 12), (x, Y + H + 6)], "ln"); f.text((X + 202 + x) / 2, Y + H + 26, "the image file system", "s c")
f.box("y", 730, Y, 56, H, 2); f.text(758, Y + 20, "device", "s c"); f.text(758, Y + 34, "tree", "s c")
f.arrow([(170, 66), (208, 66)], "ln", "a")
# flow boxes
def step(cls, x, y, w, h, title, ls):
    f.box(cls, x, y, w, h); f.text(x + w / 2, y + 20, title, "b c")
    for i, t in enumerate(ls): f.text(x + w / 2, y + 38 + i * 13, t, "s c")
step("k", 20, 170, 220, 110, "1   startup, on CPU 0", ["EL1, MMU off, x0 = device tree", "RAM size and CPU count from the tree", "sets up the GIC and the timer", "starts CPUs 1-3 (PSCI)", "builds the kernel's page tables"])
step("k", 290, 170, 220, 110, "2  the kernel", ["_start: a stack, a cleared .bss", "kmain(): memory, interrupts, clock,", "scheduler; calls in the other CPUs", "starts process 1's threads", "becomes CPU 0's idle thread"])
step("k", 560, 170, 220, 110, "3  process 1", ["serves the image's files", "runs the boot script:", "servers, the network driver,", "then sh on the console", ""])
f.arrow([(240, 215), (288, 215)], "ln", "a"); f.text(264, 207, "x0", "ms c")
f.arrow([(510, 215), (558, 215)], "ln", "a")
# numbers: which step runs or reads which part
sx, sw = pos["startup"]; kx, kw = pos["kernel"]; bx, bw = pos["boot"]; fx, fw = pos["files: servers,"]
f.num(sx + 14, Y + 12, 1, "#2c5d9b"); f.num(kx + 14, Y + 12, 2, "#2c5d9b"); f.num(bx + 12, Y + 12, 3, "#2c5d9b"); f.num(fx + 12, Y + 12, 3, "#2c5d9b"); f.num(784, Y, 1, "#2c5d9b")
f.text(400, 140, "The numbers say which step runs or reads each part. Startup and the kernel run where they lie, in the image.", "s c")
# system page
f.box("y", 150, 330, 500, 40); f.text(400, 347, "the system page: the machine, described, with the callouts", "b c"); f.text(400, 362, "memory ranges, CPUs, interrupt controller, timer, console; kept for the kernel's whole life", "s c")
f.arrow([(130, 280), (130, 350), (148, 350)], "lnk", "ak"); f.text(98, 320, "writes", "s c")
f.arrow([(400, 328), (400, 282)], "lnk", "ak"); f.text(424, 312, "reads", "s c")
f.arrow([(652, 350), (670, 350), (670, 282)], "lnk", "ak"); f.text(700, 320, "reads", "s c")
f.save("ch05-virt-boot.svg")

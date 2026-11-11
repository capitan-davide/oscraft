from svg import Fig
f = Fig(800, 380)
W = 240
def col(i, title, sub):
    x = 20 + i * (W + 20)
    f.text(x + W / 2, 26, title, "h c"); f.text(x + W / 2, 42, sub, "s c")
    f.box("frame", x, 54, W, 250)
    f.line([(x, 170), (x + W, 170)], "lnr", True)
    return x
YU, YK = 66, 182   # user area top, kernel area top
# 1: all in handler
x = col(0, "All in the handler", "simple kernels, old drivers")
f.box("u", x + 20, YU + 20, W - 40, 44); f.text(x + W / 2, YU + 39, "threads wait until the", "s c"); f.text(x + W / 2, YU + 52, "handler has finished", "s c")
f.box("k", x + 20, YK + 6, W - 40, 100); f.text(x + W / 2, YK + 30, "interrupt handler", "b c")
f.lines(x + W / 2, YK + 50, ["acknowledge the device,", "move the data,", "run the protocol"], "s c", 14)
# 2: handler + kernel thread
x = col(1, "Handler and kernel thread", "most monolithic kernels")
f.box("u", x + 20, YU + 20, W - 40, 44); f.text(x + W / 2, YU + 39, "threads of higher priority", "s c"); f.text(x + W / 2, YU + 52, "than the driver's keep running", "s c")
f.box("k", x + 10, YK + 6, 102, 100); f.text(x + 61, YK + 30, "handler", "b c"); f.lines(x + 61, YK + 50, ["quiet the", "device, wake", "the thread"], "s c", 14)
f.box("g", x + 128, YK + 6, 102, 100); f.text(x + 179, YK + 30, "thread", "b c"); f.lines(x + 179, YK + 50, ["the real work,", "scheduled by", "priority"], "s c", 14)
f.arrow([(x + 112, YK + 56), (x + 128, YK + 56)])
# 3: microkernel
x = col(2, "Thread in the driver's process", "a microkernel")
f.box("g", x + 20, YU, W - 40, 82); f.text(x + W / 2, YU + 20, "driver process: its thread", "b c"); f.lines(x + W / 2, YU + 38, ["serves the device through", "registers mapped into the process,", "scheduled by priority"], "s c", 14)
f.box("k", x + 20, YK + 28, W - 40, 62); f.text(x + W / 2, YK + 50, "kernel's handler", "b c"); f.lines(x + W / 2, YK + 66, ["mask the interrupt, wake the thread"], "s c", 14)
f.arrow([(x + W / 2, YK + 28), (x + W / 2, YU + 82)])
for i in range(3):
    xx = 20 + i * (W + 20)
    f.text(xx + 6, 166, "user mode", "s red"); f.text(xx + 6, 182, "kernel mode", "s red")
f.lines(20, 326, ["From left to right, less and less code runs where it cannot be preempted, and less driver code runs with the kernel's privileges.",
                  "In the last design a faulty driver can harm only its own process, and the kernel's handler is the same few steps for every device."], "s", 15)
f.save("ch15-where.svg")

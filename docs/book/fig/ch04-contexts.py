from svg import Fig
f = Fig(800, 345)
X0 = 120
segs = [  # (width, class, title lines, stack, irqs, may block)
    (100, "u", ["program A,", "user mode"],            "A's user stack",   "unmasked", "-"),
    (100, "r", ["interrupt", "handler"],               None,               "masked",   "no"),
    (100, "u", ["program A,", "user mode"],            "A's user stack",   "unmasked", "-"),
    (115, "k", ["kernel call", "made by A"],           None,               "masked",   "yes: it does, here"),
    (115, "k", ["a thread of", "process 1"],           "its kernel stack", "unmasked", "yes"),
    (115, "n", ["idle loop"],                          "CPU 0's boot stack", "unmasked", "no"),
]
rows = {"run": 40, "stack": 120, "irq": 180, "block": 240}
f.text(X0 - 12, rows["run"] + 28, "CPU 0 runs", "b e")
f.text(X0 - 12, rows["stack"] + 22, "on the stack", "b e")
f.text(X0 - 12, rows["irq"] + 22, "interrupts", "b e")
f.text(X0 - 12, rows["block"] + 22, "may it block?", "b e")
x = X0
xs = []
for w, cls, title, stack, irq, blk in segs:
    xs.append(x)
    f.box(cls, x, rows["run"], w - 4, 50, 3)
    if len(title) == 2:
        f.text(x + w / 2 - 2, rows["run"] + 22, title[0], "s c"); f.text(x + w / 2 - 2, rows["run"] + 36, title[1], "s c")
    else:
        f.text(x + w / 2 - 2, rows["run"] + 29, title[0], "s c")
    if stack:
        f.box("w", x, rows["stack"], w - 4, 34, 3); f.text(x + w / 2 - 2, rows["stack"] + 21, stack, "s c")
    f.box("y" if irq == "masked" else "w", x, rows["irq"], w - 4, 34, 3); f.text(x + w / 2 - 2, rows["irq"] + 21, irq, "s c")
    c = {"no": "r", "-": "w"}.get(blk, "g")
    f.box(c, x, rows["block"], w - 4, 34, 3); f.text(x + w / 2 - 2, rows["block"] + 21, blk, "s c")
    x += w
# A's kernel stack: under the handler, and under the kernel call
f.box("wk", xs[1], rows["stack"], 96, 34, 3); f.text(xs[1] + 48, rows["stack"] + 16, "A's kernel stack,", "s c"); f.text(xs[1] + 48, rows["stack"] + 29, "borrowed", "s c")
f.box("wk", xs[3], rows["stack"], 111, 34, 3); f.text(xs[3] + 55, rows["stack"] + 21, "A's kernel stack", "s c")
# events on top
f.arrow([(xs[1], 14), (xs[1], rows["run"] - 2)], "lnr", "ar"); f.text(xs[1] + 6, 18, "a device interrupts", "s red")
f.arrow([(xs[3], 14), (xs[3], rows["run"] - 2)], "lnk", "ak"); f.text(xs[3] + 6, 18, "A calls the kernel", "s blue")
f.arrow([(xs[4], 14), (xs[4], rows["run"] - 2)], "ln", "a"); f.text(xs[4] + 6, 18, "A blocks: switch", "s")
f.arrow([(xs[5], 14), (xs[5], rows["run"] - 2)], "ln", "a"); f.text(xs[5] + 6, 18, "nothing is ready", "s")
f.arrow([(X0, 300), (X0 + 641, 300)], "ln", "a"); f.text(X0 + 647, 304, "time", "s")
f.text(X0, 322, "Only code with a stack of its own can be set aside and resumed.", "s")
f.text(X0, 336, "The handler has none, and the idle loop is what runs when everything else waits.", "s")
f.save("ch04-contexts.svg")

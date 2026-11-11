from svg import Fig
f = Fig(800, 400)
# left: the GIC
f.text(170, 24, "The GICv2 of QEMU's virt", "h c")
f.box("n", 20, 36, 300, 150)
f.box("w", 32, 48, 276, 58, 3)
f.text(44, 66, "distributor, at 0x08000000", "b"); f.text(44, 82, "one for the machine: per interrupt, an enable", "s"); f.text(44, 95, "bit, a priority and the CPU it goes to", "s")
f.box("w", 32, 116, 276, 58, 3)
f.text(44, 134, "CPU interface, at 0x08010000", "b"); f.text(44, 150, "one per CPU, all at the same address:", "s"); f.text(44, 163, "acknowledge, end of interrupt, priority mask", "s")
# interrupt IDs
f.text(170, 214, "Its interrupt numbers", "h c")
ids = [("0 to 15", "software-generated: one CPU", "interrupts another", "k"),
       ("16 to 31", "private to each CPU:", "its timer", "k"),
       ("32 to 287", "shared: the devices'", "interrupt lines", "n")]
y = 226
for rng, a, b, cls in ids:
    f.box(cls, 20, y, 300, 44, 3)
    f.text(34, y + 26, rng, "m")
    f.text(120, y + 19, a, "s"); f.text(120, y + 33, b, "s")
    y += 52
# right: system page entries
f.text(580, 24, "As the system page describes it", "h c")
def entry(y, title, lines, h):
    f.box("y", 400, y, 380, h)
    f.text(414, y + 20, title, "b")
    for i, (k, v) in enumerate(lines):
        f.text(424, y + 40 + i * 16, k, "m"); f.text(550, y + 40 + i * 16, v, "s")
entry(36, "interrupt entry 1: the per-CPU interrupts", [
    ("vectors", "0 to 31 on CPU 0"),
    ("stride", "0x8000: CPU n adds n × 0x8000"),
    ("id, eoi", "code fragments: acknowledge, end"),
    ("mask, unmask", "callouts: clear or set the enable bit"),
    ("config", "callout: says interrupt 0 is the IPI")], 126)
entry(176, "interrupt entry 2: the shared interrupts", [
    ("vectors", "32 to 287, the same on every CPU"),
    ("id, eoi", "code fragments"),
    ("mask, unmask", "callouts")], 94)
# examples
f.box("w", 400, 286, 380, 96, 3)
f.text(414, 306, "Vectors the kernel uses on virt", "b")
ex = [("0, 0x8000, 0x10000, 0x18000", "the IPI of CPUs 0 to 3"),
      ("27, 0x801b, 0x1001b, 0x1801b", "the timer of CPUs 0 to 3"),
      ("32 + n", "shared interrupt n: a device")]
for i, (k, v) in enumerate(ex):
    f.text(424, 326 + i * 17, k, "ms"); f.text(620, 326 + i * 17, v, "s")
# arrows
f.arrow([(320, 248), (360, 248), (360, 100), (400, 100)], "lnk", "ak")
f.arrow([(320, 300), (372, 300), (372, 112), (400, 112)], "lnk", "ak")
f.arrow([(320, 352), (384, 352), (384, 222), (400, 222)], "lnk", "ak")
f.save("ch15-gic-virt.svg")

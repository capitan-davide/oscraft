from svg import Fig
f = Fig(800, 300)
f.text(400, 24, "Physical addresses on QEMU's virt, and the pools the startup names", "h c")
X0, PX = 60, 170  # px per GiB
def gx(g): return X0 + g * PX
f.line([(gx(0), 150), (gx(4) + 30, 150)])
for g in range(5):
    f.line([(gx(g), 146), (gx(g), 154)]); f.text(gx(g), 170, f"{g} GiB", "s c")
f.box("n", gx(0), 110, PX, 36, 2); f.text(gx(0.5), 132, "devices, no RAM", "s c")
f.box("g", gx(1), 110, 2 * PX, 36, 2); f.text(gx(2), 132, "RAM: 2 GiB", "c")
# pools
f.box("y", gx(1), 60, 2 * PX, 30, 2); f.text(gx(2), 80, "/memory", "m c")
f.line([(gx(0), 200), (gx(4), 200)], "lnk"); f.line([(gx(0), 194), (gx(0), 206)], "lnk"); f.line([(gx(4), 194), (gx(4), 206)], "lnk")
f.text(gx(2), 220, "below4G: every address under 4 GiB. All the RAM is in it, so /below4G is the same memory as /memory.", "s c")
f.line([(gx(0), 244), (gx(1) - 12, 244)], "lnr"); f.line([(gx(0), 238), (gx(0), 250)], "lnr"); f.line([(gx(1) - 12, 238), (gx(1) - 12, 250)], "lnr")
f.text(gx(1) + 4, 248, "below1G: no RAM is in it, so the startup makes no such pool, and opening /below1G fails with ENOENT.", "s red")
f.text(400, 284, "The same program on a machine with RAM under 1 GiB finds a /below1G pool, smaller than /memory.", "s c")
f.save("ch34-typed-memory.svg")

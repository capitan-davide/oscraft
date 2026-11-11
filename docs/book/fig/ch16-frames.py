from svg import Fig
f = Fig(800, 430)
f.text(20, 24, "Physical addresses of QEMU's virt", "h")
# address bar
f.box("w", 20, 36, 70, 40, 2); f.text(55, 60, "...", "s c")
f.box("n", 90, 36, 70, 40, 2); f.text(125, 60, "GIC", "s c")
f.box("n", 160, 36, 70, 40, 2); f.text(195, 60, "UART", "s c")
f.box("w", 230, 36, 90, 40, 2); f.text(275, 60, "other devices", "s c")
f.box("g", 320, 36, 400, 40, 2); f.text(520, 60, "RAM, 2 GiB", "b c")
f.box("w", 720, 36, 60, 40, 2); f.text(750, 60, "...", "s c")
for x, t in [(90, "0x08000000"), (160, "0x09000000"), (320, "0x40000000"), (720, "0xC0000000")]:
    f.line([(x, 76), (x, 84)]); f.text(x, 96, t, "ms c")
# zoom
f.text(20, 122, "The RAM: what was free at boot is the allocator's (not to scale)", "h")
Y = 132
segs = [(20, 110, "n", "boot image,", "kernel, startup"), (130, 250, "g", "", ""), (380, 30, "n", "", ""), (410, 130, "g", "free range 2", ""), (540, 30, "n", "", ""), (570, 210, "g", "free range 3", "")]
for x, w, c, a, b in segs:
    f.box(c, x, Y, w, 44, 2)
    if b: f.text(x + w / 2, Y + 20, a, "s c"); f.text(x + w / 2, Y + 33, b, "s c")
    elif a: f.text(x + w / 2, Y + 27, a, "s c")
# handed-out parts inside free ranges
f.box("u", 130, Y, 120, 44, 2); f.text(190, Y + 20, "range 1: handed", "s c"); f.text(190, Y + 33, "out since boot", "s c")
f.text(315, Y + 20, "range 1:", "s c"); f.text(315, Y + 33, "never used yet", "s c")
f.arrow([(250, Y + 74), (250, Y + 46)], "lnk", "ak"); f.text(250, Y + 88, "next: fresh frames are taken in order", "s c")
f.text(475, Y + 62, "in use: not the", "s c"); f.text(475, Y + 75, "allocator's", "s c")
f.line([(395, Y + 44), (440, Y + 52)], "life"); f.line([(555, Y + 44), (510, Y + 52)], "life")
# allocator structures
f.text(20, 258, "What the allocator keeps", "h")
f.box("y", 20, 270, 240, 92)
f.text(140, 290, "stack of freed frames", "b c"); f.lines(140, 307, ["a frame given back is pushed here", "and is the first to be handed out;", "room for every frame from the start"], "s c", 14)
f.box("y", 280, 270, 240, 92)
f.text(400, 290, "bitmap: one bit per frame", "b c"); f.lines(400, 307, ["set while the frame is free; searched", "for runs of adjacent frames", "(contiguous memory for devices)"], "s c", 14)
f.box("y", 540, 270, 240, 92)
f.text(660, 290, "share counts", "b c"); f.lines(660, 307, ["extra mappings of a frame after", "a copy-on-write fork(); the frame is", "free when the last one goes"], "s c", 14)
f.box("r", 20, 378, 760, 36, 3)
f.text(400, 400, "the kernel's reserve: the last 16 MiB of free frames are never given to a process, which gets ENOMEM instead", "s c")
f.save("ch16-frames.svg")

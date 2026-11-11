from svg import Fig
f = Fig(800, 360)
# left: contiguity
f.text(200, 24, "A device follows one address and a length", "h c")
f.text(308, 62, "virtual pages", "s"); f.text(368, 126, "frames", "s")
for i in range(4):
    f.box("u", 100 + i * 50, 44, 50, 26, 0); f.text(125 + i * 50, 62, f"page {i}", "s c")
phys = [0, 3, 1, 5]
for j in range(6):
    cls = "k" if j in phys else "n"
    f.box(cls, 60 + j * 50, 134, 50, 26, 0)
for i, j in enumerate(phys):
    f.text(85 + j * 50, 152, f"{i}", "s c")
    f.arrow([(125 + i * 50, 70), (85 + j * 50, 132)], "ln", "a")
f.text(200, 184, "Scattered frames: contiguous for the program,", "s c"); f.text(200, 198, "four separate pieces for the device.", "s c red")
for i in range(4):
    f.box("u", 100 + i * 50, 226, 50, 26, 0); f.text(125 + i * 50, 244, f"page {i}", "s c")
for j in range(6):
    f.box("g" if 1 <= j <= 4 else "n", 60 + j * 50, 296, 50, 26, 0)
    if 1 <= j <= 4: f.text(85 + j * 50, 314, f"{j-1}", "s c")
for i in range(4):
    f.arrow([(125 + i * 50, 252), (135 + i * 50, 294)], "lng", "ag")
f.text(200, 344, "One contiguous run: one address, one length.", "s c green")
f.line([(400, 16), (400, 344)], "life")
# right: reach
f.text(600, 24, "A device reaches only some addresses", "h c")
X, W = 470, 150
f.box("n", X, 50, W, 110, 0); f.text(X + W/2, 96, "RAM above 4 GiB", "c"); f.text(X + W/2, 112, "the device cannot", "s c"); f.text(X + W/2, 126, "name these addresses", "s c")
f.box("g", X, 160, W, 130, 0); f.text(X + W/2, 216, "RAM below 4 GiB", "c"); f.text(X + W/2, 232, "a pool the driver", "s c"); f.text(X + W/2, 246, "can ask for by name", "s c")
f.text(X - 8, 164, "4 GiB", "ms e"); f.text(X - 8, 294, "0", "ms e")
f.box("n", 670, 196, 110, 50); f.text(725, 216, "a device with", "s c"); f.text(725, 232, "32-bit addresses", "s c")
f.arrow([(670, 221), (X + W + 3, 221)], "lng", "ag")
f.arrow([(725, 196), (725, 100), (X + W + 3, 100)], "lnr", "ar", True)
f.text(600, 322, "The buffer must come from memory the device", "s c"); f.text(600, 336, "can address, whatever the program's addresses are.", "s c")
f.save("ch20-dma.svg")

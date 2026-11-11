from svg import Fig
f = Fig(800, 470)
# Row 1: the memory, two views.
f.text(400, 22, "One block of contiguous memory, here with four buffers per CPU", "h c")
X, W = 40, 180
for c in range(4):
    x = X + c * W
    for b in range(4):
        f.box("y", x + b * 45, 62, 45, 30, 0)
        f.text(x + b * 45 + 22, 81, str(c * 4 + b), "s c")
    f.text(x + 90, 108, f"CPU {c} writes these", "s c")
f.box("k", 40, 30, 200, 22); f.text(140, 45, "mapped in the kernel", "s c")
f.box("u", 560, 30, 200, 22); f.text(660, 45, "mapped, shared, in the logger", "s c")
f.arrow([(140, 52), (140, 61)], "lnk", "ak"); f.arrow([(660, 52), (660, 61)])
# Row 2: one buffer.
f.text(400, 142, "One buffer: 16 KiB", "h c")
cells = [(40, 130, "n", "header", "208 bytes"), (170, 80, "k", "TIME", "slot 0"), (250, 80, "k", "BUFFER", "slot 1"),
         (330, 250, "y", "events", "16-byte slots, 1011 in a buffer"), (580, 100, "k", "BUFFER_END", "last used slot"), (680, 80, "w", "", "")]
for x, w, cls, a, b in cells:
    f.box(cls, x, 150, w, 44, 0)
    if a:
        f.text(x + w // 2, 169, a, "c"); f.text(x + w // 2, 185, b, "s c")
f.text(105, 212, "flags, slots used,", "s c"); f.text(105, 225, "sequence number", "s c")
f.text(250, 212, "the full clock value,", "s c"); f.text(250, 225, "the buffer's number", "s c")
f.text(680, 212, "written when the buffer", "s c"); f.text(680, 225, "is full or tracing stops", "s c")
# Row 3: one slot.
f.line([(330, 194), (120, 290)], "life"); f.line([(390, 194), (680, 290)], "life")
f.text(400, 266, "One slot: four 32-bit words", "h c")
sl = [(120, 200, "k", "header"), (320, 120, "y", "time, low half"), (440, 120, "w", "data 0"), (560, 120, "w", "data 1")]
for x, w, cls, t in sl:
    f.box(cls, x, 290, w, 34, 0); f.text(x + w // 2, 311, t, "c")
# header bits
f.line([(120, 324), (60, 362)], "life"); f.line([(320, 324), (540, 362)], "life")
bits = [(60, 90, "31-30", "part"), (150, 110, "29-24", "CPU"), (260, 80, "23-15", ""), (340, 90, "14-10", "class"), (430, 110, "9-0", "event")]
for x, w, b, t in bits:
    f.box("wk" if t else "n", x, 362, w, 34, 0); f.text(x + w // 2, 376, "bits " + b, "s c"); f.text(x + w // 2, 390, t, "s c b")
f.text(60, 416, "part: this slot is a whole event, or the beginning, a continuation or the end of a longer one.", "s")
# combine event
f.text(60, 448, "A longer event:", "s")
for i, t in enumerate(["begin", "continue", "end"]):
    f.box("y", 170 + i * 130, 432, 130, 26, 0); f.text(235 + i * 130, 449, t + ": 2 words", "s c")
f.text(570, 449, "same header and time in each slot", "s")
f.save("ch35-buffer-layout.svg")

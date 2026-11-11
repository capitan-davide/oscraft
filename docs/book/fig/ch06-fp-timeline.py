from svg import Fig
f = Fig(800, 330)
x0 = 150
f.text(x0 - 12, 64, "the CPU runs", "e"); f.text(x0 - 12, 134, "thread A's FP state is", "e")
segs = [(150, 320, "g", "thread A"), (320, 470, "u", "thread B"), (470, 640, "g", "thread A"), (640, 770, "u", "thread B")]
for a, b, c, t in segs:
    f.box(c, a, 40, b - a, 40, 3); f.text((a + b) // 2, 64, t, "c")
# FP uses inside A's runs
for x in (210, 540):
    f.arrow([(x, 16), (x, 39)], "lnr", "ar"); f.text(x + 8, 24, "uses floating point", "s red")
st = [(150, 210, "y", "saved"), (210, 320, "g", "in the registers"), (320, 540, "y", "saved in A's thread structure"), (540, 640, "g", "in the registers"), (640, 770, "y", "saved")]
for a, b, c, t in st:
    f.box(c, a, 110, b - a, 40, 3); f.text((a + b) // 2, 134, t, "s c")
ev = [(210, 1, "#aa2222"), (320, 2, "#333333"), (470, 3, "#333333"), (540, 4, "#aa2222"), (640, 5, "#333333")]
for x, n, col in ev:
    f.line([(x, 80), (x, 110)], "life"); f.line([(x, 150), (x, 172)], "life")
    f.num(x, 182, n, col)
f.arrow([(150, 206), (770, 206)]); f.text(770, 222, "time", "s e")
notes = [
    (1, "#aa2222", "A's first floating-point instruction traps. The kernel loads A's state and lets the instruction run again."),
    (2, "#333333", "Switch to B. A used the registers, so they are saved. Floating-point instructions are set to trap."),
    (3, "#333333", "Switch back to A. B never used the registers: nothing is saved and nothing is loaded."),
    (4, "#aa2222", "A's next floating-point instruction traps, and its state is loaded again."),
    (5, "#333333", "Switch to B: A's state is saved, as at 2."),
]
for i, (n, col, t) in enumerate(notes):
    y = 246 + i * 17
    f.num(48, y - 4, n, col); f.text(64, y, t, "s")
f.save("ch06-fp-timeline.svg")

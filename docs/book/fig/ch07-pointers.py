from svg import Fig
f = Fig(800, 330)
f.text(40, 24, "pointer arguments of five calls", "h")
f.text(40, 224, "the caller's address space", "s")
y = 158
segs = [(40, 200, "u", "data", "read, write"), (200, 330, "w", "nothing mapped", ""), (330, 450, "u", "code", "read only"), (450, 560, "u", "stack", "read, write"), (600, 760, "k", "kernel memory", "")]
for a, b, c, t, s in segs:
    f.box(c, a, y, b - a, 46, 2)
    f.text((a + b) // 2, y + (20 if s else 28), t, "c")
    if s: f.text((a + b) // 2, y + 36, s, "s c")
f.line([(580, 140), (580, 222)], "lnr", True); f.text(580, 238, "end of user addresses", "s c red")
ptrs = [
    (90, 70, True, ["read 64 bytes", "from here"], ["mapped and readable:", "the call goes on"]),
    (235, 60, False, ["read from", "here"], ["no memory:", "error"]),
    (360, 60, False, ["write the", "result here"], ["not writable:", "error"]),
    (505, 68, False, ["read 1 MiB", "from here"], ["runs past the", "mapping: error"]),
    (650, 60, False, ["read from", "here"], ["not the", "caller's: error"]),
]
for x, w, ok, top, bot in ptrs:
    cls, m, col = ("lng", "ag", "green") if ok else ("lnr", "ar", "red")
    f.text(x, 52, top[0], "s"); f.text(x, 65, top[1], "s")
    f.arrow([(x, 74), (x, y - 2)], cls, m)
    f.line([(x, 120), (min(x + w, 770), 120)], cls)
    f.line([(min(x + w, 770), 114), (min(x + w, 770), 126)], cls)
    f.text(x, 264, bot[0], "s " + col); f.text(x, 278, bot[1], "s " + col)
f.text(400, 312, "The kernel checks the whole range, and the kind of access, before it touches a byte: a bad pointer fails the call, never the kernel.", "s c")
f.save("ch07-pointers.svg")

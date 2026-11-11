from svg import Fig
f = Fig(800, 300)
f.box("k", 20, 60, 120, 50); f.text(80, 81, "a point in", "c"); f.text(80, 97, "the kernel", "c")
f.arrow([(140, 85), (178, 85)])
f.box("w", 180, 60, 120, 50); f.text(240, 81, "tracing", "c"); f.text(240, 97, "running?", "c")
f.arrow([(300, 85), (338, 85)]); f.text(319, 78, "yes", "s c")
f.box("w", 340, 60, 120, 50); f.text(400, 81, "this event's", "c"); f.text(400, 97, "bit set?", "c")
f.arrow([(460, 85), (498, 85)]); f.text(479, 78, "yes", "s c")
f.box("y", 500, 34, 280, 196)
f.text(640, 54, "the slow path: emit()", "b c")
steps = ["mask interrupts on this CPU", "take this CPU's trace lock", "start a buffer if there is none", "write the event's 16-byte slots", "unlock, restore interrupts"]
for i, s in enumerate(steps):
    y = 66 + i * 32
    f.box("wk", 520, y, 240, 24); f.text(640, y + 16, s, "s c")
    if i:
        f.arrow([(640, y - 8), (640, y - 1)], "lnk", "ak")
# The "no" exits.
f.arrow([(240, 110), (240, 168)], "lng", "ag"); f.text(248, 142, "no", "s green")
f.arrow([(400, 110), (400, 168)], "lng", "ag"); f.text(408, 142, "no", "s green")
f.box("g", 180, 170, 280, 44); f.text(320, 189, "return at once", "b c"); f.text(320, 205, "cost: one or two loads and a test", "s c")
f.text(400, 262, "A call site whose arguments are costly to gather makes the same test first. What the slow path never does:", "s c")
f.text(400, 278, "wait for a buffer, or wake a thread. With no free buffer the event is dropped.", "s c")
f.save("ch35-fast-path.svg")

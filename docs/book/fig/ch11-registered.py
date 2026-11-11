from svg import Fig
f = Fig(800, 420)
cx, kx, sx = 120, 400, 690
f.box("u", cx - 70, 16, 140, 32); f.text(cx, 37, "client", "c")
f.box("k", kx - 90, 16, 180, 32); f.text(kx, 37, "kernel", "c")
f.box("u", sx - 70, 16, 140, 32); f.text(sx, 37, "server", "c")
for x in (cx, kx, sx): f.line([(x, 48), (x, 300)], "life")
# the client's table, in the kernel
f.box("y", kx - 90, 84, 180, 78, 3)
f.text(kx, 100, "the client's registered events", "s c b")
f.text(kx - 80, 119, "0  (free)", "ms")
f.text(kx - 80, 135, "1  pulse, code 5 → server", "ms")
f.text(kx - 80, 151, "2  signal 30   → anyone", "ms")
y = 74
f.arrow([(cx, y), (kx - 92, y + 30)], "lnk", "ak"); f.num(cx + 20, y - 4, 1, "#2c5d9b")
f.text(cx + 36, y - 12, "register: this event,", "s"); f.text(cx + 36, y, "deliverable by the server", "s")
y = 150
f.arrow([(kx - 92, y - 10), (cx + 2, y + 10)], "lng", "ag"); f.text(cx + 40, y + 22, "handle 1", "s green")
y = 196
f.arrow([(cx, y), (sx - 2, y)], "lnk", "ak"); f.num(cx + 20, y - 14, 2, "#2c5d9b")
f.text(cx + 36, y - 8, "request: \"notify me with handle 1\"", "s")
f.box("w", kx + 110, y + 6, 0, 0)
y = 244
f.arrow([(sx, y), (kx + 2, y)], "lnk", "ak"); f.num(sx - 20, y - 14, 3, "#2c5d9b")
f.text(sx - 36, y - 8, "deliver to the client: handle 1", "s e")
y = 282
f.arrow([(kx, y), (cx + 2, y)], "lnk", "ak", True); f.num(kx - 20, y - 14, 4, "#2c5d9b")
f.text(kx - 36, y - 8, "entry 1, server allowed: the pulse", "s e")
# refused
f.line([(20, 316), (780, 316)], "life")
f.text(400, 338, "What the server cannot do", "h c")
f.box("u", sx - 70, 354, 140, 44); f.text(sx, 372, "a faulty or", "s c"); f.text(sx, 386, "hostile server", "s c")
f.box("r", kx - 90, 354, 180, 44); f.text(kx, 372, "not a handle: refused", "s c b"); f.text(kx, 386, "a handle not for it: refused", "s c")
f.box("u", cx - 70, 354, 140, 44); f.text(cx, 372, "gets nothing it", "s c"); f.text(cx, 386, "did not ask for", "s c")
f.arrow([(sx - 72, 376), (kx + 92, 376)], "lnr", "ar")
f.text((sx + kx) // 2 + 12, 368, "deliver: \"signal 9\"", "s c red")
f.save("ch11-registered.svg")

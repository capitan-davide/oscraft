from svg import Fig
f = Fig(800, 420)
A, B, C = 130, 400, 670
f.box("k", A - 100, 20, 200, 34); f.text(A, 34, "CPU 2: the kernel", "c"); f.text(A, 48, "emitting events", "s c")
f.box("k", B - 100, 20, 200, 34); f.text(B, 34, "the kernel, on any CPU", "c"); f.text(B, 48, "at a safe point", "s c")
f.box("r", C - 100, 20, 200, 34); f.text(C, 34, "the logger's thread", "c"); f.text(C, 48, "bound to CPU 2", "s c")
for x in (A, B, C):
    f.line([(x, 54), (x, 380)], "life")
# 1 logger waits
f.num(C - 62, 92, 1); f.text(C - 76, 89, "waits for a", "s e"); f.text(C - 76, 102, "full buffer", "s e")
f.box("y", C - 45, 70, 90, 150, 3); f.text(C, 140, "blocked", "s c"); f.text(C, 153, "(no CPU time)", "s c")
# 2 buffer fills
f.box("g", A - 45, 70, 90, 60, 3); f.text(A, 97, "writes slots", "s c"); f.text(A, 110, "into buffer 8", "s c")
f.num(A + 62, 142, 2); f.text(A + 76, 139, "buffer 8 is full: end marker,", "s"); f.text(A + 76, 152, "queued; \"something is pending\"", "s")
f.box("g", A - 45, 160, 90, 200, 3); f.text(A, 250, "goes on in", "s c"); f.text(A, 263, "buffer 9", "s c")
# 3 kick
f.num(B - 62, 196, 3, "#2c5d9b"); f.text(B - 76, 193, "end of a kernel call,", "s e"); f.text(B - 76, 206, "or the clock tick", "s e")
f.box("k", B - 45, 180, 90, 34, 3); f.text(B, 201, "kick()", "m c")
f.arrow([(B + 45, 220), (C - 47, 220)], "lnk", "ak"); f.text((B + C) // 2, 213, "wakes the thread", "s c")
# 4 logger takes it
f.box("g", C - 45, 228, 90, 92, 3); f.text(C, 262, "gets index 8,", "s c"); f.text(C, 275, "writes the buffer", "s c"); f.text(C, 288, "to its file", "s c")
f.num(C - 62, 262, 4)
f.arrow([(C - 47, 320), (A + 47, 320)], "ln", "a", True); f.num((A + C) // 2 - 110, 306, 5); f.text((A + C) // 2 - 96, 310, "clears the buffer's header: buffer 8 is free again", "s")
f.box("y", C - 45, 328, 90, 40, 3); f.text(C, 345, "waits for", "s c"); f.text(C, 358, "the next", "s c")
f.text(400, 402, "Time runs downwards. The writer never waits for the logger; if the logger falls behind and no buffer is free, events are dropped.", "s c")
f.save("ch35-handover.svg")

from svg import Fig
f = Fig(800, 584)
A, Q, B = 150, 400, 650
for cls, x, w, t, sub in [("k", A, 240, "CPU 0", "startup, then the kernel"), ("n", Q, 150, "QEMU", "answers PSCI calls"), ("k", B, 240, "CPU n", "powered off at first")]:
    f.box(cls, x - w / 2, 12, w, 40); f.text(x, 29, t, "b c"); f.text(x, 44, sub, "s c"); f.line([(x, 52), (x, 572)], "life")
def note(x, y, lines, cls="wk", w=220):
    h = 8 + 13 * len(lines)
    f.box(cls, x - w / 2, y, w, h, 3)
    for i, t in enumerate(lines): f.text(x, y + 15 + i * 13, t, "s c")
    return y + h
# phase labels
f.text(400, 72, "in startup", "s c green"); f.line([(20, 76), (780, 76)], "life")
note(A, 86, ["hvc: CPU_ON(n, entry)"])
f.arrow([(A + 110, 97), (Q - 2, 97)], "ln", "a"); f.arrow([(Q + 2, 110), (B - 112, 110)], "ln", "a"); f.text((Q + B) / 2 - 20, 104, "starts it at entry", "s c")
note(B, 118, ["EL1, MMU off: describes itself in the", "system page (its ID, its GIC interface,", "its timer); says it is done; waits"])
f.arrow([(B - 110, 174), (A + 2, 174)], "lng", "ag", True); f.text(Q + 70, 168, "done", "s c")
note(A, 184, ["next CPU, the same way; then the", "system page is written to its place"])
f.arrow([(A + 110, 224), (B - 2, 224)], "ln", "a"); f.text(Q - 60, 218, "go on", "s c")
note(B, 232, ["turns its MMU on, with its own", "top-level table; jumps to smp_spin,", "a callout in the system page"])
f.box("y", B - 7, 286, 14, 136, 2); f.text(B + 14, 346, "spins on", "s"); f.text(B + 14, 359, "smp.start_addr", "ms")
f.arrow([(B - 9, 298), (A + 2, 298)], "lng", "ag", True); f.text(Q + 70, 292, "there", "s c")
note(A, 308, ["all CPUs wait: jump into the kernel"])
f.text(424, 350, "in the kernel", "s c green"); f.line([(20, 354), (B - 12, 354)], "life")
note(A, 364, ["kmain() ... start_secondaries():", "map a stack for one CPU;", "start_addr = ap_start; wake them"])
f.arrow([(A + 110, 416), (B - 9, 416)], "lnk", "ak"); f.text(Q, 410, "one CPU swaps the word back to 0, and goes", "s c")
note(B, 426, ["ap_start: takes the stack", "kmain_ap(): its per-CPU data, vectors,", "MMU settings, scheduler, clock, IPIs"])
e = note(B, 484, ["online = true; becomes its idle thread"], "g")
f.arrow([(B - 110, 496), (A + 2, 496)], "lng", "ag", True); f.text(Q, 490, "the count of online CPUs grows", "s c")
note(A, 506, ["next CPU, until all are online", "(gives up after one second)"])
f.save("ch05-smp.svg")

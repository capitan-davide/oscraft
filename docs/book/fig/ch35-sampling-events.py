from svg import Fig
f = Fig(800, 330)
X0, X1 = 130, 770
# What happened on one CPU.
f.text(120, 64, "what the CPU ran", "e b")
segs = [(130, 300, "g", "thread A"), (300, 318, "r", ""), (318, 372, "u", "B"), (372, 560, "g", "thread A"),
        (560, 640, "k", "C"), (640, 770, "g", "thread A")]
for a, b, cls, name in segs:
    f.box(cls, a, 44, b - a, 32, 0)
    if name:
        f.text((a + b) // 2, 64, name, "c")
f.text(309, 36, "interrupt", "s c red")
f.line([(309, 39), (309, 44)], "lnr")
# Sampling.
f.text(120, 142, "sampling", "e b")
f.text(120, 157, "look at fixed intervals", "s e")
f.line([(X0, 138), (X1, 138)], "life")
seen = ["A", "A", "A", "A", "C", "A"]
for i, s in enumerate(seen):
    x = 165 + i * 110
    f.line([(x, 76), (x, 126)], "life")
    f.num(x, 138, s, "#333333")
# Event tracing.
f.text(120, 236, "event tracing", "e b")
f.text(120, 251, "record every change", "s e")
f.line([(X0, 232), (X1, 232)], "life")
evs = [(300, "interrupt", 268, "e"), (318, "B runs", 268, ""), (372, "A runs", 284, "c"), (560, "C runs", 268, "c"), (640, "A runs", 284, "c")]
for x, label, ly, anchor in evs:
    f.line([(x, 76), (x, 222)], "life")
    f.box("y", x - 4, 224, 8, 16, 1)
    f.line([(x, 240), (x, ly - 12)], "ln")
    f.text(x - 3 if anchor == "e" else (x + 3 if anchor == "" else x), ly, label, "s " + anchor if anchor else "s")
f.text(450, 314, "Five records, each with its time: the order of events and every duration can be rebuilt exactly.", "s c")
f.o.append('  <rect x="170" y="164" width="560" height="18" fill="#ffffff"/>\n')
f.text(450, 176, "Sees A five times and C once. The interrupt and thread B ran between two looks and left no trace.", "s c")
f.save("ch35-sampling-events.svg")

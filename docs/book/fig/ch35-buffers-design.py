from svg import Fig
f = Fig(800, 400)
# Left: one shared buffer.
f.text(200, 24, "One buffer for all CPUs", "h c")
for i in range(4):
    y = 44 + i * 34
    f.box("k", 30, y, 70, 24); f.text(65, y + 16, f"CPU {i}", "c")
    f.arrow([(100, y + 12), (176, 107)], "lnk", "ak")
f.box("r", 178, 92, 34, 30); f.text(195, 111, "lock", "s c")
f.box("y", 212, 92, 160, 30); f.text(292, 111, "events of all CPUs", "s c")
f.text(200, 196, "Every event takes the same lock: tracing", "s c")
f.text(200, 210, "makes the CPUs wait for each other, and", "s c")
f.text(200, 224, "changes the timing it is meant to show.", "s c")
f.line([(400, 14), (400, 236)], "life")
# Right: a buffer per CPU.
f.text(600, 24, "A buffer per CPU", "h c")
for i in range(4):
    y = 44 + i * 34
    f.box("k", 430, y, 70, 24); f.text(465, y + 16, f"CPU {i}", "c")
    f.arrow([(500, y + 12), (538, y + 12)], "lnk", "ak")
    f.box("y", 540, y, 120, 24); f.text(600, y + 16, "its own events", "s c")
    f.arrow([(660, y + 12), (698, 95 + i * 8)], "ln", "a", True)
f.box("u", 700, 80, 90, 54); f.text(745, 97, "reader:", "s c"); f.text(745, 111, "merges", "s c"); f.text(745, 125, "by time", "s c")
f.text(600, 196, "A CPU writes only its own buffer and never", "s c")
f.text(600, 210, "waits for another. Each record carries a time,", "s c")
f.text(600, 224, "so the reader can put them back in one order.", "s c")
# Bottom: what to do when a buffer is full.
f.line([(20, 246), (780, 246)], "life")
f.text(400, 268, "When the buffers are full", "h c")
def strip(x0, y, cells):
    for i, (cls, t) in enumerate(cells):
        f.box(cls, x0 + i * 54, y, 54, 26, 0); f.text(x0 + i * 54 + 27, y + 17, t, "s c")
f.text(200, 292, "keep the beginning: stop writing", "b c")
strip(30, 302, [("y", "1"), ("y", "2"), ("y", "3"), ("y", "4"), ("y", "5")])
f.text(308, 319, "6, 7, 8: dropped", "s red")
f.text(200, 358, "Right when a reader empties buffers as fast as they fill.", "s c")
f.text(600, 292, "keep the end: overwrite the oldest", "b c")
strip(465, 302, [("g", "6"), ("g", "7"), ("g", "8"), ("y", "4"), ("y", "5")])
f.arrow([(654, 346), (654, 330)], "lng", "ag"); f.text(654, 358, "next write", "s c green")
f.text(600, 384, "A flight recorder: the last moments before a stop.", "s c")
f.save("ch35-buffers-design.svg")

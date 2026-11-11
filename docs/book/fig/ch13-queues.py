from svg import Fig
f = Fig(800, 470)
def cpu(x0, n, entries, tick, hrt, note):
    f.box("frame", x0, 14, 360, 300)
    f.text(x0 + 14, 36, f"CPU {n}", "h")
    # timeout queue
    f.text(x0 + 14, 62, "timeout queue: blocked threads, by deadline", "s")
    for i, (d, t) in enumerate(entries):
        x = x0 + 14 + i * 112
        f.box("y", x, 70, 104, 40, 3); f.text(x + 52, 87, d, "ms c"); f.text(x + 52, 102, t, "s c")
        if i:
            f.arrow([(x - 8, 90), (x - 1, 90)])
    # the two deadlines
    f.box("wk", x0 + 14, 150, 160, 46); f.text(x0 + 94, 169, "next tick", "c"); f.text(x0 + 94, 186, tick, "ms c")
    f.box("wk", x0 + 186, 150, 160, 46); f.text(x0 + 266, 169, "next high-res timer", "c"); f.text(x0 + 266, 186, hrt, "ms c")
    f.text(x0 + 94, 138, "on the grid", "s c")
    f.text(x0 + 266, 138, "an exact time", "s c")
    f.arrow([(x0 + 94, 196), (x0 + 94, 222), (x0 + 150, 222), (x0 + 150, 248)], "lnk", "ak")
    f.arrow([(x0 + 266, 196), (x0 + 266, 222), (x0 + 210, 222), (x0 + 210, 248)], "lnk", "ak")
    f.text(x0 + 180, 216, "the earlier", "s c blue")
    f.box("n", x0 + 90, 250, 180, 48); f.text(x0 + 180, 270, "comparator", "b c"); f.text(x0 + 180, 287, note, "s c")
cpu(20, 0, [("t = 12 ms", "thread A"), ("t = 40 ms", "thread B"), ("t = 900 ms", "thread C")], "t = 8 ms", "none", "set to 8 ms")
cpu(420, 1, [("t = 25 ms", "thread D")], "t = 8 ms", "t = 7.35 ms", "set to 7.35 ms")
# the timers
f.text(20, 344, "timers: one list for the system, each timer marked with the CPU that armed it", "s")
T = [("k", "timer 0 of pid 12", "cpu 0, t = 30 ms", "every 10 ms"),
     ("k", "alarm of pid 31", "cpu 0, t = 2 s", "once"),
     ("k", "timer 0 of pid 44", "cpu 1, t = 7.35 ms", "high-resolution"),
     ("k", "timer 1 of pid 44", "cpu 1, t = 60 ms", "once")]
for i, (c, a, b, d) in enumerate(T):
    x = 20 + i * 194
    f.box(c, x, 354, 178, 62); f.text(x + 89, 372, a, "c"); f.text(x + 89, 389, b, "ms c"); f.text(x + 89, 405, d, "s c")
f.text(400, 440, "A tick on CPU n expires the due entries of queue n and the due timers marked n. No CPU looks through another's.", "s c")
f.text(400, 456, "Times are of the monotonic clock; the tick is 1 ms.", "s c")
f.save("ch13-queues.svg")

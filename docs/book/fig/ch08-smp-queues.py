from svg import Fig
f = Fig(800, 330)
def cpu(x, y, n, prio, cls="g"):
    f.text(x + 40, y - 6, f"CPU {n}", "s c")
    f.box(cls, x, y, 80, 40); f.text(x + 40, y + 17, "runs", "s c"); f.text(x + 40, y + 32, str(prio), "b c")
def thr(x, y, prio, cls="y"):
    f.box(cls, x, y, 60, 24, 3); f.text(x + 30, y + 16, str(prio), "c")
# left: a queue per CPU
f.text(200, 24, "One queue per CPU", "h c")
runs = [30, 12, 10, 5]; queues = [[25, 20], [], [8], []]
for i in range(4):
    x = 30 + i * 90
    cpu(x, 60, i, runs[i], "r" if i == 3 else "g")
    f.line([(x + 40, 100), (x + 40, 116)], "life")
    for j, p in enumerate(queues[i]):
        thr(x + 10, 118 + j * 30, p, "r" if (i == 0 and j == 0) else "y")
    if not queues[i]:
        f.text(x + 40, 134, "(empty)", "s c")
f.text(200, 206, "Each CPU takes the best thread of its own queue.", "s c")
f.text(200, 222, "No shared lock, but CPU 3 runs priority 5 while", "s c")
f.text(200, 238, "a thread of priority 25 waits on CPU 0:", "s c")
f.text(200, 254, "the queues must be balanced, and until they", "s c")
f.text(200, 270, "are, priorities hold only within one CPU.", "s c")
f.line([(400, 16), (400, 290)], "life")
# right: one queue
f.text(600, 24, "One queue for all CPUs", "h c")
runs = [30, 25, 20, 12]
for i in range(4):
    cpu(430 + i * 90, 60, i, runs[i])
f.box("frame", 470, 118, 260, 44)
f.text(462, 144, "ready", "s e")
for j, p in enumerate([10, 8, 5]):
    thr(490 + j * 80, 128, p)
f.text(600, 206, "Every CPU takes the best thread of the one queue.", "s c")
f.text(600, 222, "The CPUs always run the highest-priority ready", "s c")
f.text(600, 238, "threads of the whole system, at the price of a", "s c")
f.text(600, 254, "lock that all CPUs share and of threads that", "s c")
f.text(600, 270, "move between CPUs (unless an affinity mask pins them).", "s c")
f.text(400, 314, "Numbers are priorities. Yellow: ready, waiting for a CPU. Red: the pair that breaks the priority rule.", "s c")
f.save("ch08-smp-queues.svg")

from svg import Fig
f = Fig(800, 356)
f.box("frame", 150, 14, 630, 270)
f.text(166, 36, "one process", "h")
f.box("y", 170, 50, 590, 44); f.text(465, 69, "pending for the process", "b c"); f.text(465, 85, "SIGTERM", "ms c")
T = [("thread 1", "blocks SIGTERM", "cannot take it", "n"), ("thread 2", "blocks nothing", "takes SIGTERM", "g"), ("thread 3", "blocks SIGTERM", "cannot take it", "n")]
for i, (n, m, r, c) in enumerate(T):
    x = 170 + i * 200
    f.box("u", x, 140, 190, 60); f.text(x + 95, 160, n, "b c"); f.text(x + 95, 177, "mask: " + m, "s c")
    f.box("y", x + 20, 214, 150, 46, 3); f.text(x + 95, 232, "pending for this thread", "s c")
    f.text(x + 95, 249, "SIGUSR1" if i == 2 else "none", "ms c")
f.arrow([(465, 94), (465, 138)], "lng", "ag"); f.text(473, 120, "any thread that does not block it", "s green")
f.arrow([(60, 72), (168, 72)]); f.text(20, 54, "kill(pid, SIGTERM)", "ms")
f.arrow([(20, 300), (665, 300), (665, 262)], "ln", "a")
f.text(20, 318, "pthread_kill(thread 3, SIGUSR1)", "ms")
f.text(400, 344, "Actions belong to the process. Each thread has its own mask, and its own set of signals sent to it alone.", "s c")
f.save("ch14-threads.svg")

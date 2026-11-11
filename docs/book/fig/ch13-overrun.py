from svg import Fig
f = Fig(800, 300)
Y = 120
X0, STEP = 80, 110
f.text(20, 26, "A periodic timer whose event is a signal", "h")
f.arrow([(40, Y), (770, Y)]); f.text(770, Y + 18, "time", "s e")
labels = ["signal sent", "overrun = 1", "overrun = 2", "signal sent", "signal sent"]
cls = ["green", "red", "red", "green", "green"]
for i in range(5):
    x = X0 + i * STEP
    f.line([(x, Y - 14), (x, Y + 14)], "lnk")
    f.text(x, Y - 22, f"expiry {i + 1}", "s c blue")
    f.text(x, Y + 32, labels[i], "s c " + cls[i])
# pending bar
xa = X0; xb = X0 + 2 * STEP + 50
f.box("y", xa, Y + 46, xb - xa, 30, 3); f.text((xa + xb) / 2, Y + 65, "the signal is pending: the thread blocks it", "s c")
f.line([(xb, Y - 40), (xb, Y + 76)], "lng", True)
f.text(xb, Y - 46, "the thread unblocks the signal", "s c green")
f.box("g", xb, Y + 46, 40, 30, 3)
f.text(xb + 50, Y + 58, "the handler runs:", "s"); f.text(xb + 50, Y + 72, "timer_getoverrun() reads 2", "s")
f.text(400, 250, "A timer never has two of its signals pending. An expiry that finds the last one still waiting is counted, not queued,", "s c")
f.text(400, 266, "so a slow receiver falls behind by a number, not by a growing queue.", "s c")
f.save("ch13-overrun.svg")

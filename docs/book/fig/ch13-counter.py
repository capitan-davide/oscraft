from svg import Fig
f = Fig(800, 420)
# the hardware
f.text(20, 24, "The hardware", "h")
f.box("n", 20, 40, 220, 64); f.text(130, 64, "counter", "b c"); f.text(130, 81, "counts up at a fixed rate", "s c"); f.text(130, 95, "reading it tells the time", "s c")
f.box("n", 300, 40, 220, 64); f.text(410, 64, "comparator", "b c"); f.text(410, 81, "holds one value,", "s c"); f.text(410, 95, "set by the kernel", "s c")
f.arrow([(240, 72), (298, 72)])
f.box("r", 600, 50, 180, 44); f.text(690, 69, "interrupt", "b c"); f.text(690, 85, "when counter >= value", "s c")
f.arrow([(520, 72), (598, 72)], "lnr", "ar")
# two timelines
def axis(y, title, sub):
    f.text(20, y - 72, title, "h"); f.text(20, y - 56, sub, "s")
    f.arrow([(60, y), (770, y)])
    f.text(775, y + 4, "", "s")
def deadline(x, y, name):
    f.line([(x, y - 30), (x, y + 2)], "lnr", True); f.text(x, y - 34, name, "s c red")
X0, STEP = 90, 60
# periodic
Y1 = 232
axis(Y1, "Periodic tick", "The comparator is set one period ahead, every time. Each tick, the kernel looks for deadlines that have passed.")
for i in range(11):
    x = X0 + i * STEP
    f.line([(x, Y1 - 12), (x, Y1 + 12)], "lnk")
d1, d2 = X0 + 2 * STEP + 22, X0 + 6 * STEP + 40
deadline(d1, Y1, "deadline A"); deadline(d2, Y1, "deadline B")
f.arrow([(d1, Y1 + 22), (X0 + 3 * STEP - 2, Y1 + 22)], "lng", "ag"); f.text(X0 + 3 * STEP, Y1 + 40, "A noticed", "s c green")
f.arrow([(d2, Y1 + 22), (X0 + 7 * STEP - 2, Y1 + 22)], "lng", "ag"); f.text(X0 + 7 * STEP, Y1 + 40, "B noticed", "s c green")
f.text(X0 + 10 * STEP, Y1 + 40, "11 interrupts", "s c blue")
# one-shot
Y2 = 362
axis(Y2, "One-shot (tickless)", "The comparator is set to the next deadline itself. Nothing happens in between.")
d1b, d2b = d1, d2
deadline(d1b, Y2, "deadline A"); deadline(d2b, Y2, "deadline B")
for x in (d1b, d2b):
    f.line([(x, Y2 - 12), (x, Y2 + 12)], "lnk")
f.text(d1b, Y2 + 28, "A handled", "s c green"); f.text(d2b, Y2 + 28, "B handled", "s c green")
f.text(X0 + 10 * STEP, Y2 + 28, "2 interrupts", "s c blue")
f.text(400, 410, "Blue marks are timer interrupts. Time runs to the right.", "s c")
f.save("ch13-counter.svg")

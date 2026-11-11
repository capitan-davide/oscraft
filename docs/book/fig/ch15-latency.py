from svg import Fig
f = Fig(800, 300)
Y = 110
f.arrow([(30, Y + 80), (780, Y + 80)]); f.text(775, Y + 98, "time", "s e")
# CPU activity bar
f.text(30, Y - 8, "What the CPU is doing", "s b")
f.box("u", 30, Y, 250, 40, 3); f.text(155, Y + 17, "another thread, or kernel code", "s c"); f.text(155, Y + 31, "that has interrupts masked", "s c")
f.box("k", 280, Y, 110, 40, 3); f.text(335, Y + 17, "interrupt", "s c"); f.text(335, Y + 31, "handler", "s c")
f.box("u", 390, Y, 130, 40, 3); f.text(455, Y + 17, "a thread of", "s c"); f.text(455, Y + 31, "higher priority", "s c")
f.box("g", 520, Y, 200, 40, 3); f.text(620, Y + 17, "the driver's thread", "s c"); f.text(620, Y + 31, "serves the device", "s c")
# events
def ev(x, lab1, lab2):
    f.line([(x, 46), (x, Y + 86)], "life")
    f.text(x, 26, lab1, "s c"); f.text(x, 39, lab2, "s c")
ev(130, "the device", "raises its line")
ev(280, "the handler", "starts")
ev(520, "the driver's", "thread runs")
# intervals
def span(x0, x1, y, t, cls="lnr", m="ar"):
    f.arrow([(x0 + 2, y), (x1 - 2, y)], cls, m, start=True)
    f.text((x0 + x1) / 2, y - 6, t, "s c b")
span(130, 280, Y + 64, "interrupt latency")
span(280, 520, Y + 64, "scheduling latency")
f.text(205, Y + 112, "as long as the longest stretch", "s c"); f.text(205, Y + 125, "with interrupts masked", "s c")
f.text(400, Y + 112, "the handler itself, then every thread", "s c"); f.text(400, Y + 125, "of higher priority than the driver's", "s c")
f.text(30, 280, "A real-time system must bound both intervals. Keeping the handler short bounds the first; priorities decide the second.", "s")
f.save("ch15-latency.svg")

from svg import Fig
f = Fig(800, 330)
def row(y, title, sub):
    f.text(20, y, title, "h"); f.text(20, y + 16, sub, "s")
    f.arrow([(20, y + 86), (780, y + 86)])
# lazy
y = 26
row(y, "Backed on demand", "the kernel only notes the mapping; a frame is found when a page is first touched")
f.box("k", 40, y + 34, 40, 36, 3); f.text(60, y + 56, "mmap", "s c")
f.box("g", 80, y + 34, 130, 36, 3); f.text(145, y + 56, "program runs", "s c")
xs = [210, 330, 480, 640]
for i, x in enumerate(xs):
    last = i == len(xs) - 1
    f.box("r", x, y + 34, 46, 36, 3); f.text(x + 23, y + 56, "fault", "s c")
    if not last:
        nx = xs[i + 1]
        f.box("g", x + 46, y + 34, nx - x - 46, 36, 3); f.text((x + 46 + nx) / 2, y + 56, "runs", "s c")
f.box("g", 686, y + 34, 84, 36, 3); f.text(728, y + 56, "runs", "s c")
f.text(233, y + 102, "first touch", "s c"); f.text(353, y + 102, "first touch", "s c"); f.text(503, y + 102, "first touch", "s c")
f.text(663, y + 102, "first touch: and if no frame is free now?", "s c red")
# eager
y = 186
row(y, "Backed when mapped", "every page has its frame before mmap() returns, or mmap() fails")
f.box("k", 40, y + 34, 200, 36, 3); f.text(140, y + 50, "mmap: allocate and", "s c"); f.text(140, y + 63, "fill every page", "s c")
f.box("g", 240, y + 34, 530, 36, 3); f.text(505, y + 56, "program runs: no fault, at any time", "s c")
f.text(40, y + 102, "the whole cost, at a moment the program chose", "s")
f.text(770, y + 124, "time", "s e")
f.save("ch16-backing.svg")

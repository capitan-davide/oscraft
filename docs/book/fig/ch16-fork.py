from svg import Fig
f = Fig(800, 470)
def proc(x, y, title, cells):
    f.text(x + 60, y - 8, title, "b c")
    for i, (lab, c) in enumerate(cells):
        f.box(c, x, y + i * 34, 120, 28, 3); f.text(x + 60, y + i * 34 + 18, lab, "s c")
def frames(x, y, cells):
    for i, (lab, c) in enumerate(cells):
        if lab is None: continue
        f.box(c, x, y + i * 34, 110, 28, 2); f.text(x + 55, y + i * 34 + 18, lab, "s c")
# default
f.text(20, 24, "fork() as it is by default: private pages are copied before fork() returns", "h")
Y = 56
pc = [("private page", "u"), ("private page", "u"), ("shared object", "y"), ("program code", "g")]
proc(40, Y, "parent", pc); proc(640, Y, "child", pc)
f.text(400, Y - 8, "frames", "b c")
frames(280, Y, [("frame", "u"), ("frame", "u"), (None, None), (None, None)])
frames(420, Y, [("copy", "u"), ("copy", "u"), (None, None), (None, None)])
frames(345, Y, [(None, None), (None, None), ("object's frame", "y"), ("image's page", "g")])
for i in range(2):
    yy = Y + i * 34 + 14
    f.arrow([(160, yy), (280, yy)]); f.arrow([(640, yy), (530, yy)])
    f.arrow([(390, yy), (420, yy)], "lnk", "ak")
for i in (2, 3):
    yy = Y + i * 34 + 14
    f.arrow([(160, yy), (345, yy)]); f.arrow([(640, yy), (455, yy)])
f.text(400, Y + 154, "Afterwards neither process can fault on its memory. The cost is paid inside fork().", "s c")
f.line([(10, 232), (790, 232)], "life")
# cow
f.text(20, 256, "The opt-in alternative: copy-on-write", "h")
Y2 = 290
proc(40, Y2, "parent", [("private page", "u"), ("private page", "u")])
proc(640, Y2, "child", [("private page", "u"), ("private page", "u")])
f.text(400, Y2 - 8, "frames", "b c")
frames(345, Y2, [("frame, 2 users", "u"), ("frame, 2 users", "u")])
for i in range(2):
    yy = Y2 + i * 34 + 14
    f.arrow([(160, yy), (345, yy)], "ln", "a", True); f.arrow([(640, yy), (455, yy)], "ln", "a", True)
f.text(252, Y2 + 6, "read-only", "s c"); f.text(548, Y2 + 6, "read-only", "s c")
f.box("r", 40, Y2 + 90, 720, 58, 3)
f.text(52, Y2 + 110, "first write to a page, by either process", "b")
f.text(52, Y2 + 126, "The write faults. The kernel gives the writer its own copy of that page (or, if the other user has gone, makes the page writable)", "s")
f.text(52, Y2 + 140, "and the write is done again. fork() is fast; the cost comes later, one fault at a time, at moments nobody chose.", "s")
f.save("ch16-fork.svg")

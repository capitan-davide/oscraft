from svg import Fig
f = Fig(800, 400)
def space(x, title, cls, pages):
    f.text(x + 85, 26, title, "h c")
    f.box("frame", x, 36, 170, 330)
    ys = {}
    for name, y, c, lab in pages:
        f.box(c, x + 15, y, 140, 30, 3); f.text(x + 85, y + 19, lab, "s c")
        ys[name] = y + 15
    return ys
A = space(20, "Process A's addresses", "u", [
    ("k", 50, "k", "the kernel"), ("st", 120, "u", "stack"), ("no", 170, "w", "nothing mapped"),
    ("sh", 220, "y", "shared buffer"), ("d", 270, "u", "data"), ("c", 320, "u", "code")])
B = space(610, "Process B's addresses", "u", [
    ("k", 50, "k", "the kernel"), ("st", 120, "u", "stack"), ("sh", 170, "y", "shared buffer"),
    ("no", 220, "w", "nothing mapped"), ("d", 270, "u", "data"), ("c", 320, "u", "code")])
# frames
f.text(400, 26, "Physical memory: frames", "h c")
fr = [("k", "k", "kernel"), ("a_st", "u", "A"), ("b_d", "u", "B"), ("sh", "y", "shared"), ("a_d", "u", "A"),
      ("free1", "w", "free"), ("b_st", "u", "B"), ("code", "g", "program file"), ("free2", "w", "free")]
FY = {}
for i, (n, c, lab) in enumerate(fr):
    y = 40 + i * 36
    f.box(c, 340, y, 120, 30, 2); f.text(400, y + 19, lab, "s c"); FY[n] = y + 15
def link(side, ys, name, frame, cls="ln", m="a"):
    if side == "A": f.arrow([(175, ys[name]), (340, FY[frame])], cls, m)
    else: f.arrow([(625, ys[name]), (460, FY[frame])], cls, m)
link("A", A, "k", "k", "lnk", "ak"); link("B", B, "k", "k", "lnk", "ak")
link("A", A, "st", "a_st"); link("A", A, "d", "a_d"); link("A", A, "sh", "sh", "lng", "ag"); link("A", A, "c", "code")
link("B", B, "st", "b_st"); link("B", B, "d", "b_d"); link("B", B, "sh", "sh", "lng", "ag"); link("B", B, "c", "code")
f.text(400, 382, "The same address means different frames in A and in B. A frame is shared only where both have it mapped.", "s c")
f.save("ch16-virtual.svg")

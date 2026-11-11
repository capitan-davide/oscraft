from svg import Fig
f = Fig(800, 360)
def panel(x, title, rows, note):
    f.text(x + 120, 24, title, "h c")
    y = 40
    for cls, lines, h in rows:
        f.box(cls, x, y, 240, h)
        ty = y + h / 2 + 4 - (len(lines) - 1) * 7.5
        for i, t in enumerate(lines):
            f.text(x + 120, ty + i * 15, t, "c" if i == 0 else "s c")
        y += h + 8
    for i, t in enumerate(note):
        f.text(x + 120, 300 + i * 15, t, "s c")
panel(20, "In a library", [
    ("r", ["the foreign program"], 44),
    ("k", ["a replacement C library", "its functions, built on the host's calls"], 60),
    ("k", ["the host kernel", "one system-call interface"], 114),
], ["The program's own library is replaced.", "Calls made without it are not caught."])
panel(280, "At the kernel boundary", [
    ("r", ["the foreign program"], 44),
    ("r", ["its own C library, unchanged"], 60),
    ("k", ["a second system-call interface", "converts numbers and structures"], 30 + 16),
    ("k", ["the host kernel's core"], 60),
], ["Nothing in user space is replaced.", "The kernel keeps two interfaces."])
panel(540, "A whole machine", [
    ("r", ["the foreign program"], 44),
    ("r", ["its own C library, unchanged"], 60),
    ("r", ["its own kernel, unchanged"], 46),
    ("k", ["a virtual machine on the host"], 60),
], ["Everything foreign is kept.", "The two systems share nothing."])
f.line([(270, 16), (270, 340)], "life"); f.line([(530, 16), (530, 340)], "life")
f.text(400, 352, "Red: the foreign system's code. Blue: the host's.", "s c")
f.save("ch32-emulation-layers.svg")

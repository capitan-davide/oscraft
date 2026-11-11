from svg import Fig
f = Fig(800, 330)
ops = ["receive its interrupt", "map its device's registers", "open any file", "kill any process", "set the clock", "map any physical memory", "become any user"]
need = [True, True, False, False, False, False, False]
def panel(x0, title, sub, split):
    f.text(x0 + 185, 26, title, "h c"); f.text(x0 + 185, 42, sub, "s c")
    f.box("u", x0, 56, 110, 60); f.text(x0 + 55, 82, "network", "c"); f.text(x0 + 55, 98, "driver", "c")
    for i, (o, n) in enumerate(zip(ops, need)):
        y = 56 + i * 34
        if n: cls, note = "g", "needed"
        elif split: cls, note = "n", "not held"
        else: cls, note = "r", "also held"
        f.box(cls, x0 + 150, y, 220, 26, 3); f.text(x0 + 160, y + 17, o, "s")
        col = {"g": "green", "r": "red", "n": ""}[cls]
        f.text(x0 + 364, y + 17, note, f"s e {col}")
        if n or not split:
            a, m = ("lng", "ag") if n else ("lnr", "ar")
            f.arrow([(x0 + 110, 86), (x0 + 130, 86), (x0 + 130, y + 13), (x0 + 148, y + 13)], a, m)
    if split:
        f.lines(x0 + 55, 150, ["a bug here", "can misuse", "two things"], "s c", 14)
    else:
        f.lines(x0 + 55, 150, ["a bug here", "can misuse", "everything"], "s c red", 14)
panel(20, "One privilege: the superuser", "the driver must run as root to do two things", False)
f.line([(400, 16), (400, 300)], "life")
panel(420, "One privilege per kind of operation", "the driver is given the two it needs", True)
f.save("ch17-root.svg")

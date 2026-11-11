from svg import Fig
f = Fig(800, 300)
def panel(x0, title, shared):
    f.text(x0 + 190, 24, title, "h c")
    f.box("u", x0 + 20, 40, 150, 46); f.text(x0 + 95, 60, "process A", "c"); f.text(x0 + 95, 76, "writes 'x'", "s c")
    f.box("u", x0 + 210, 40, 150, 46); f.text(x0 + 285, 60, "process B", "c"); f.text(x0 + 285, 76, "reads 'x'" if shared else "does not see 'x'", "s c")
    if shared:
        f.box("y", x0 + 100, 180, 180, 50); f.text(x0 + 190, 202, "the object's page frames", "c"); f.text(x0 + 190, 218, "one copy, now holding 'x'", "s c")
        f.arrow([(x0 + 95, 86), (x0 + 150, 178)], "lnk", "ak"); f.arrow([(x0 + 285, 86), (x0 + 230, 178)], "lnk", "ak")
        f.text(x0 + 190, 262, "Both page tables point at the same frames.", "s c")
    else:
        f.box("wu", x0 + 20, 112, 150, 40, 3); f.text(x0 + 95, 130, "A's own frames", "s c"); f.text(x0 + 95, 144, "now holding 'x'", "s c")
        f.box("wu", x0 + 210, 112, 150, 40, 3); f.text(x0 + 285, 136, "B's own frames", "s c")
        f.arrow([(x0 + 95, 86), (x0 + 95, 110)], "lnk", "ak"); f.arrow([(x0 + 285, 86), (x0 + 285, 110)], "lnk", "ak")
        f.box("y", x0 + 100, 196, 180, 44); f.text(x0 + 190, 215, "the object", "c"); f.text(x0 + 190, 230, "unchanged", "s c")
        f.arrow([(x0 + 160, 194), (x0 + 110, 154)], "ln", "a", True); f.arrow([(x0 + 220, 194), (x0 + 270, 154)], "ln", "a", True)
        f.text(x0 + 190, 272, "Each mapping starts as a copy of the object.", "s c")
panel(10, "MAP_SHARED", True)
f.line([(400, 16), (400, 284)], "life")
panel(410, "MAP_PRIVATE", False)
f.save("ch20-shared-private.svg")

from svg import Fig
f = Fig(800, 340)
f.text(250, 24, "Three sources for one fact, from cheapest to most certain", "h c")
rows = [
    ("y", "1. The documentation", "says what the designers intended", "fast to read; silent or wrong in places"),
    ("y", "2. A measurement", "a program run on the reference", "shows what happens, for the cases tried"),
    ("y", "3. The binary itself", "its code, read with a disassembler", "shows why, and the cases nobody tried"),
]
y = 44
for cls, a, b, c in rows:
    f.box(cls, 20, y, 460, 62); f.text(34, y + 22, a, "b"); f.text(34, y + 40, b, "s"); f.text(34, y + 55, c, "s")
    y += 74
f.arrow([(500, 60), (500, 250)]); f.text(506, 160, "when the one above", "s"); f.text(506, 175, "does not settle it", "s")
# clean room wall
f.line([(640, 40), (640, 300)], "lnr", True)
f.text(640, 30, "the clean-room line", "s c red")
f.box("g", 660, 60, 130, 88); f.text(725, 82, "may cross", "b c"); f.lines(725, 100, ["numbers, layouts,", "return values,", "what a function does"], "s c", 14)
f.box("r", 660, 170, 130, 88); f.text(725, 192, "never crosses", "b c"); f.lines(725, 210, ["code, in any form,", "and the text of", "the documentation"], "s c", 14)
f.text(400, 322, "Whatever the source, what is taken from it is a fact about an interface, written down in one's own words.", "s c")
f.save("ch34-sources.svg")

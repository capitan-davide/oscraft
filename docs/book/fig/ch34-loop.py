from svg import Fig
f = Fig(800, 330)
steps = [
    (30, 40, "u", "1  A question", ["What does a QNX program", "see in this case?"]),
    (300, 40, "y", "2  A probe", ["a small C program that", "prints what it observes"]),
    (570, 40, "r", "3  Run on both kernels", ["tools/qnx/probe.py --diff:", "procnto's output, OsCraft's"]),
    (570, 190, "k", "4  Change OsCraft", ["until the two outputs", "are identical"]),
    (300, 190, "y", "5  Keep it as a test", ["the program into tests/qnx,", "its output into target.tests"]),
    (30, 190, "g", "6  Both suites", ["make test QNX=1 and", "make qnx-test pass it"]),
]
for x, y, cls, t, lines in steps:
    f.box(cls, x, y, 200, 90); f.text(x + 100, y + 26, t, "b c")
    for i, l in enumerate(lines): f.text(x + 100, y + 50 + i * 16, l, "s c")
f.arrow([(230, 85), (298, 85)]); f.arrow([(500, 85), (568, 85)])
f.arrow([(670, 130), (670, 188)])
f.arrow([(568, 235), (502, 235)]); f.arrow([(298, 235), (232, 235)])
f.arrow([(620, 190), (620, 132)], "ln", "a", True); f.text(612, 164, "again", "s e")
f.text(400, 310, "The expected values of a QNX test are procnto's measured results, so the same test passes on both kernels.", "s c")
f.save("ch34-loop.svg")

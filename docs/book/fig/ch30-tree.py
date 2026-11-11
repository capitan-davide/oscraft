from svg import Fig
f = Fig(800, 380)
# the host
f.box("k", 250, 16, 300, 96); f.text(400, 36, "the host", "h c")
f.box("wk", 266, 46, 268, 26, 3); f.text(400, 64, "host controller: starts every transfer", "s c")
f.box("wk", 266, 78, 268, 26, 3); f.text(400, 96, "root hub: the controller's own ports", "s c")
for i, x in enumerate([310, 370, 430, 490]):
    f.box("n", x - 9, 104, 18, 14, 2)
# tier 1
f.box("u", 70, 176, 150, 44); f.text(145, 195, "keyboard", "c"); f.text(145, 210, "address 1", "s c")
f.box("y", 300, 170, 200, 50); f.text(400, 188, "hub", "b c"); f.text(400, 203, "address 2, four ports", "s c")
for x in [340, 380, 420, 460]:
    f.box("n", x - 9, 213, 18, 14, 2)
f.box("n", 590, 176, 150, 44); f.text(665, 195, "a free port", "c"); f.text(665, 210, "nothing attached", "s c")
f.arrow([(310, 118), (145, 174)]); f.arrow([(370, 118), (392, 168)]); f.line([(490, 118), (665, 176)], "ln", True)
# tier 2
f.box("u", 150, 290, 190, 44); f.text(245, 309, "network adapter", "c"); f.text(245, 324, "address 3", "s c")
f.box("u", 460, 290, 190, 44); f.text(555, 309, "storage device", "c"); f.text(555, 324, "address 4", "s c")
f.arrow([(340, 226), (255, 288)]); f.arrow([(420, 226), (545, 288)])
f.text(30, 60, "the root", "s"); f.text(30, 160, "tier 1", "s"); f.text(30, 280, "tier 2", "s")
f.line([(20, 140), (780, 140)], "life"); f.line([(20, 258), (780, 258)], "life")
f.text(400, 362, "One host, a tree of hubs, devices at the leaves. A device never talks unless the host has just asked it to.", "s c")
f.save("ch30-tree.svg")

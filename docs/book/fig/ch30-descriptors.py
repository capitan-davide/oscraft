from svg import Fig
f = Fig(800, 430)
f.box("y", 20, 20, 230, 92); f.text(135, 40, "device descriptor", "b c")
f.lines(34, 60, ["vendor and product IDs", "device class", "number of configurations"], "s", 15)
f.box("g", 20, 150, 230, 66); f.text(135, 170, "endpoint 0", "b c"); f.lines(135, 188, ["control, both directions;", "exists before any configuration"], "s c", 14)
f.box("y", 300, 20, 480, 390); f.text(540, 40, "configuration 1", "b c"); f.text(540, 56, "the host selects one: SET_CONFIGURATION", "s c")
f.arrow([(250, 60), (298, 60)])
# interface 0
f.box("w", 320, 76, 440, 100, 3); f.text(540, 94, "interface 0: class Communications", "b c"); f.text(540, 110, "what kind of network adapter this is, and its Ethernet address", "s c")
f.box("g", 420, 124, 240, 40, 3); f.text(540, 141, "endpoint 1 IN: interrupt", "c"); f.text(540, 156, "link up, link down", "s c")
# interface 1
f.box("w", 320, 192, 440, 204, 3); f.text(540, 210, "interface 1: class Data", "b c"); f.text(540, 226, "two alternate settings; the host selects one: SET_INTERFACE", "s c")
f.box("n", 336, 240, 190, 140, 3); f.text(431, 260, "alternate setting 0", "c"); f.lines(431, 300, ["no endpoints:", "the adapter is idle"], "s c", 15)
f.box("wk", 554, 240, 190, 140, 3); f.text(649, 260, "alternate setting 1", "c")
f.box("g", 566, 274, 166, 40, 3); f.text(649, 291, "endpoint 2 IN: bulk", "c"); f.text(649, 306, "received packets", "s c")
f.box("g", 566, 326, 166, 40, 3); f.text(649, 343, "endpoint 2 OUT: bulk", "c"); f.text(649, 358, "packets to send", "s c")
f.text(135, 262, "A device describes itself", "s c"); f.text(135, 277, "with these structures.", "s c")
f.text(135, 300, "The host reads them over", "s c"); f.text(135, 315, "endpoint 0 and picks drivers", "s c"); f.text(135, 330, "by the interfaces' classes.", "s c")
f.save("ch30-descriptors.svg")

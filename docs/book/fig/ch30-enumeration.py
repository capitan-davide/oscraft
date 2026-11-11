from svg import Fig
f = Fig(800, 500)
hx, px, dx = 150, 400, 600
f.box("k", hx - 90, 16, 180, 34); f.text(hx, 31, "host", "b c"); f.text(hx, 44, "USB core and hub driver", "s c")
f.box("y", px - 60, 16, 120, 34); f.text(px, 31, "hub", "b c"); f.text(px, 44, "the port", "s c")
f.box("u", dx - 60, 16, 120, 34); f.text(dx, 31, "new device", "b c"); f.text(dx, 44, "just plugged in", "s c")
for x in (hx, px, dx): f.line([(x, 50), (x, 480)], "life")
def msg(y, a, b, t, n, cls="lnk", m="ak", col="#2c5d9b", dash=False):
    f.arrow([(a, y), (b, y)], cls, m, dash)
    lo, hi = min(a, b), max(a, b)
    f.num(lo + 16, y - 12, n, col); f.text(lo + 32, y - 8, t, "s")
msg(90, dx, px, "the port senses a device", 1, "ln", "a", "#333333")
msg(126, px, hx, "status change, on the hub's interrupt endpoint", 2, "ln", "a", "#333333")
msg(166, hx, px, "reset the port", 3)
msg(206, hx, dx, "GET_DESCRIPTOR(device), to address 0", 4)
msg(240, dx, hx, "18 bytes: IDs, class, packet size of endpoint 0", 5, "lng", "ag", "#3e7b34", True)
msg(280, hx, dx, "SET_ADDRESS(3)", 6)
msg(320, hx, dx, "GET_DESCRIPTOR(configuration), to address 3", 7)
msg(354, dx, hx, "the configuration, its interfaces and endpoints", 8, "lng", "ag", "#3e7b34", True)
msg(394, hx, dx, "SET_CONFIGURATION(1)", 9)
f.box("g", hx - 110, 420, 220, 44, 3); f.text(hx, 438, "match drivers to interfaces", "c"); f.text(hx, 453, "by class, or by vendor and product", "s c")
# device states
sx = 690
f.text(sx + 45, 78, "device state", "s c")
for y, h, t, c in [(84, 74, "Powered", "n"), (166, 106, "Default", "y"), (280, 106, "Address", "y"), (394, 70, "Configured", "g")]:
    f.box(c, sx, y, 90, h - 6, 3); f.text(sx + 45, y + (h - 6) / 2 + 4, t, "s c")
f.save("ch30-enumeration.svg")

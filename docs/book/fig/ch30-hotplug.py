from svg import Fig
f = Fig(800, 500)
cols = [(75, "n", "adapter", "plugged in"), (205, "g", "USB core", "usbd: hub driver"), (335, "k", "/dev/usbd", "usbd: generation"), (475, "k", "watcher", "netd: usbcl's thread"), (610, "g", "USB core", "netd: hub driver"), (735, "g", "if_cdce", "netd: class driver")]
for x, c, t, s in cols:
    f.box(c, x - 60, 14, 120, 36); f.text(x, 30, t, "b c"); f.text(x, 43, s, "s c"); f.line([(x, 50), (x, 470)], "life")
f.line([(405, 8), (405, 480)], "frame")
f.text(398, 66, "usbd", "s e"); f.text(412, 66, "netd", "s")
X = [c[0] for c in cols]
def msg(y, a, b, t, n, cls="lnk", m="ak", col="#2c5d9b", dash=False):
    f.arrow([(X[a], y), (X[b], y)], cls, m, dash)
    lo = min(X[a], X[b])
    f.num(lo + 14, y - 12, n, col); f.text(lo + 28, y - 8, t, "s")
f.box("y", X[3] - 50, 76, 100, 150, 3); f.text(X[3], 140, "blocked in", "s c"); f.text(X[3], 153, "USBDIOCWAIT", "ms c"); f.text(X[3], 168, "generation 4", "s c")
msg(100, 0, 1, "port change", 1, "ln", "a", "#333333")
f.box("g", X[1] - 62, 110, 124, 40, 3); f.text(X[1], 127, "enumerates:", "s c"); f.text(X[1], 141, "address, descriptors", "s c")
msg(182, 1, 2, "event: ATTACH", 2)
f.box("y", X[2] - 50, 190, 100, 26, 3); f.text(X[2], 207, "generation 5", "s c")
msg(240, 2, 3, "the wait returns 5", 3, "lng", "ag", "#3e7b34")
msg(276, 3, 2, "USBDIOCLIST", 4)
f.box("y", X[3] - 64, 286, 128, 40, 3); f.text(X[3], 303, "a new device:", "s c"); f.text(X[3], 317, "root port 1 connected", "s c")
msg(352, 3, 4, "port change", 5)
msg(388, 4, 2, "USBDIOCCTRL: descriptors (SET_ADDRESS stays local)", 6)
msg(424, 4, 5, "probe, attach", 7)
f.box("g", X[5] - 40, 434, 80, 26, 3); f.text(X[5], 451, "ue0", "m c")
f.text(400, 494, "The device is enumerated twice: by usbd on the real bus, then by netd's core on the client controller's root hub.", "s c")
f.save("ch30-hotplug.svg")

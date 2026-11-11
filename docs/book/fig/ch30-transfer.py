from svg import Fig
f = Fig(800, 500)
cols = [(75, "g", "if_cdce", "netd: class driver"), (205, "k", "usbcl", "netd: pipe start"), (335, "k", "endpoint thread", "netd: usbcl"), (475, "k", "/dev/usbd", "usbd: serving thread"), (610, "g", "USB core, xhci", "usbd: the kept pipe"), (735, "n", "adapter", "the device")]
for x, c, t, s in cols:
    f.box(c, x - 60, 14, 120, 36); f.text(x, 30, t, "b c"); f.text(x, 43, s, "s c"); f.line([(x, 50), (x, 470)], "life")
f.line([(405, 8), (405, 480)], "frame")
f.text(398, 66, "netd", "s e"); f.text(412, 66, "usbd", "s")
X = [c[0] for c in cols]
def msg(y, a, b, t, n, cls="lnk", m="ak", col="#2c5d9b", dash=False, off=0):
    f.arrow([(X[a], y), (X[b], y)], cls, m, dash)
    lo = min(X[a], X[b])
    f.num(lo + 14 + off, y - 12, n, col); f.text(lo + 28 + off, y - 8, t, "s")
msg(96, 0, 1, "start the transfer", 1)
msg(132, 1, 2, "queue it, signal", 2)
msg(160, 1, 0, "returns at once", 3, "ln", "a", "#333333", True)
msg(204, 2, 3, "USBDIOCXFER: one frame", 4)
f.box("y", X[2] - 42, 212, 84, 150, 3); f.text(X[2], 282, "REPLY-", "s c"); f.text(X[2], 295, "blocked", "s c")
msg(240, 3, 4, "start the pipe's transfer", 5)
f.box("y", X[3] - 42, 248, 84, 78, 3); f.text(X[3], 284, "waits for", "s c"); f.text(X[3], 297, "the callback", "s c")
msg(272, 4, 5, "IN", 6)
msg(300, 5, 4, "a packet", 7, "lng", "ag", "#3e7b34", True)
msg(334, 4, 3, "callback: transferred", 8, "lng", "ag", "#3e7b34", True, 36)
msg(370, 3, 2, "reply: the data, its length", 9, "lng", "ag", "#3e7b34", False, 36)
msg(416, 2, 0, "usbd_transfer_done(): the driver's callback runs, as after a controller's interrupt", 10, "lng", "ag", "#3e7b34")
f.text(400, 494, "Steps 1 to 3 never block the class driver. Steps 4 to 9 are one synchronous message between the two servers.", "s c")
f.save("ch30-transfer.svg")

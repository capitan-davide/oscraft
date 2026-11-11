from svg import Fig
f = Fig(800, 440)
f.box("u", 250, 14, 300, 32); f.text(400, 35, "MsgSendv() on a connection", "b c")
qs = [
    ("Is the sender a QNX process?", "no"),
    ("Is the server process 1?", "yes"),
    ("Is the server a native process?", "no"),
    ("Is the server qnxd itself?", "yes"),
    ("A plain read or write, or a notify request?", "yes"),
    ("Is qnxd registered and alive?", "no"),
]
y = 64
for i, (q, direct) in enumerate(qs):
    f.arrow([(400, y - 18), (400, y - 2)])
    f.box("k", 240, y, 320, 34); f.text(400, y + 21, q, "c")
    last = i == len(qs) - 1
    cont = "yes" if direct == "no" else "no"
    f.text(408, y + 47, cont, "s")
    if not last:
        f.arrow([(560, y + 17), (640, y + 17)], "lng", "ag"); f.text(600, y + 11, direct, "s c green")
    else:
        f.arrow([(240, y + 17), (160, y + 17)], "lnr", "ar"); f.text(200, y + 11, direct, "s c red")
        f.box("r", 20, y - 8, 140, 50); f.text(90, y + 13, "the send fails:", "s c"); f.text(90, y + 29, "ENOSYS", "m c")
    y += 52
f.box("g", 640, 64, 140, 242); f.text(710, 172, "straight to", "c"); f.text(710, 190, "the server", "b c"); f.text(710, 212, "nothing to", "s c"); f.text(710, 226, "translate", "s c")
f.arrow([(400, y - 18), (400, y - 2)])
f.box("y", 250, y, 300, 40); f.text(400, y + 17, "queued on qnxd's channel", "b c"); f.text(400, y + 33, "the connection is remembered for the forward", "s c")
f.save("ch33-routing.svg")

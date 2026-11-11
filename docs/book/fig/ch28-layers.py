from svg import Fig
f = Fig(800, 430)
# the layers
X, W = 150, 300
f.box("u", X, 20, W, 44); f.text(X + W/2, 39, "a program", "b c"); f.text(X + W/2, 55, "send(s, buf, n)   recv(s, buf, n)", "ms c")
rows = [
    (96,  "the socket layer", "a socket per descriptor, with a send", "buffer and a receive buffer"),
    (176, "the protocols: TCP, UDP, IP", "a control block per socket: addresses,", "ports, the connection's state"),
    (256, "the interface layer", "routing picks an interface; Ethernet", "framing; address resolution"),
    (336, "a device driver", "queues of buffers shared with", "the device"),
]
for y, t, a, b in rows:
    f.box("w", X, y, W, 60); f.text(X + W/2, y + 19, t, "b c"); f.text(X + W/2, y + 36, a, "s c"); f.text(X + W/2, y + 50, b, "s c")
f.box("n", X, 400, W, 24); f.text(X + W/2, 416, "the network device", "c")
# down and up
f.arrow([(70, 70), (70, 396)], "lnk", "ak"); f.text(70, 60, "sending", "c blue")
f.arrow([(110, 396), (110, 70)], "lng", "ag"); f.text(110, 412, "receiving", "c green")
# what the packet looks like at each boundary
PX = 500
f.text(PX + 140, 34, "what travels between the layers", "h c")
def pkt(y, parts):
    x = PX
    for cls, w, t in parts:
        f.box(cls, x, y, w, 24, 2); f.text(x + w/2, y + 16, t, "s c"); x += w
pkt(68, [("y", 120, "the program's bytes")])
pkt(148, [("y", 120, "data")])
f.text(PX + 130, 164, "a chain of buffers", "s")
pkt(228, [("k", 44, "IP"), ("k", 44, "TCP"), ("y", 120, "data")])
pkt(308, [("g", 52, "Ethernet"), ("k", 44, "IP"), ("k", 44, "TCP"), ("y", 120, "data")])
f.text(PX + 140, 364, "each layer going down puts its", "s c"); f.text(PX + 140, 378, "header in front; each layer going", "s c"); f.text(PX + 140, 392, "up takes its header off", "s c")
for y in (80, 160, 240, 320):
    f.line([(X + W, y), (PX - 6, y)], "life")
f.save("ch28-layers.svg")

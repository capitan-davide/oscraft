from svg import Fig
f = Fig(800, 500)
f.box("n", 20, 20, 150, 140); f.text(95, 40, "the device (QEMU)", "h c")
f.box("k", 190, 20, 420, 460); f.text(400, 40, "netd", "h c")
f.box("u", 630, 20, 150, 460); f.text(705, 40, "a program", "h c")
# two thread columns in netd
f.box("wk", 204, 54, 230, 414, 3); f.text(319, 72, "the driver's interrupt thread", "b c")
f.box("wk", 446, 54, 150, 414, 3); f.text(521, 72, "a serving thread", "b c")
def stage(y, h, t, lines, cls="g"):
    f.box(cls, 216, y, 206, h, 3); f.text(319, y + 16, t, "c")
    for i, l in enumerate(lines): f.text(319, y + 31 + i * 13, l, "s c")
stage(86, 58, "virtio_pci: the handler", ["reads the interrupt status,", "calls each queue's handler"])
stage(154, 72, "vtnet: the receive queue", ["takes the used chain: an mbuf,", "already filled by the device;", "offers a fresh mbuf in its place"])
stage(236, 44, "Ethernet input", ["checks the frame, strips its header"])
stage(290, 44, "IP input", ["checks the header, finds the protocol"])
stage(344, 58, "UDP input", ["finds the socket by address and port,", "appends the mbuf to its receive buffer"])
stage(412, 44, "wakes the socket's sleepers", ["and whoever polls it"])
for y0, y1 in [(144, 154), (226, 236), (280, 290), (334, 344), (402, 412)]:
    f.arrow([(319, y0), (319, y1)], "ln", "a")
# device
f.box("w", 32, 86, 126, 58, 3); f.text(95, 104, "writes the frame into", "s c"); f.text(95, 118, "a buffer of the receive", "s c"); f.text(95, 132, "queue; interrupt", "s c")
f.arrow([(158, 115), (214, 115)], "lng", "ag"); f.num(182, 101, 1, "#3e7b34")
f.text(95, 186, "through the kernel,", "s c"); f.text(95, 200, "which wakes the thread", "s c"); f.text(95, 214, "(Chapter 15)", "s c")
f.box("k", 20, 290, 150, 80); f.text(95, 310, "process 1", "h c"); f.text(95, 330, "answers: the fresh", "s c"); f.text(95, 344, "mbuf's physical", "s c"); f.text(95, 358, "address", "s c")
f.arrow([(216, 214), (180, 214), (180, 330), (172, 330)], "lnk", "ak", start=True); f.num(180, 262, 2, "#2c5d9b")
# serving thread
f.box("y", 456, 86, 130, 72, 3); f.text(521, 104, "asleep in", "s c"); f.text(521, 118, "soreceive() since the", "s c"); f.text(521, 132, "program's recv()", "s c"); f.text(521, 146, "arrived", "s c")
f.box("g", 456, 398, 130, 62, 3); f.text(521, 416, "soreceive() copies", "s c"); f.text(521, 430, "the datagram out", "s c"); f.text(521, 444, "and frees the mbuf", "s c")
f.arrow([(422, 434), (454, 434)], "lng", "ag"); f.num(438, 420, 3, "#3e7b34")
f.line([(521, 158), (521, 398)], "life")
# program
f.box("y", 642, 86, 126, 44, 3); f.text(705, 104, "REPLY-blocked", "s c"); f.text(705, 120, "in recv()", "s c")
f.box("g", 642, 408, 126, 44, 3); f.text(705, 426, "recv() returns", "s c"); f.text(705, 442, "the datagram", "s c")
f.arrow([(586, 430), (640, 430)], "lng", "ag"); f.num(613, 416, 4, "#3e7b34")
f.line([(705, 130), (705, 408)], "life")
f.text(705, 478, "", "s")
f.save("ch28-rx.svg")

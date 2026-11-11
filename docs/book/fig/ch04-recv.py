from svg import Fig
f = Fig(800, 600)
P, K, R, I, D = 70, 230, 410, 590, 745
cols = [("u", P, 110, "the program", "calls recv()"), ("k", K, 170, "the kernel", "ker/, sched, interrupt, arch/"),
        ("g", R, 150, "netd", "a receiving thread"), ("g", I, 150, "netd", "the interrupt thread"), ("n", D, 90, "virtio", "network device")]
for cls, x, w, t, sub in cols:
    f.box(cls, x - w / 2, 12, w, 40); f.text(x, 29, t, "b c"); f.text(x, 44, sub, "s c")
    f.line([(x, 52), (x, 588)], "life")
def note(x, y, lines, cls="w", w=150):
    h = 8 + 13 * len(lines)
    f.box(cls, x - w / 2, y, w, h, 3)
    for i, t in enumerate(lines): f.text(x, y + 15 + i * 13, t, "s c")
def bar(x, y0, y1, cls="y"): f.box(cls, x - 7, y0, 14, y1 - y0, 2)
# waits, as bars on the lifelines
bar(P, 84, 539); f.text(P + 12, 250, "REPLY-", "s"); f.text(P + 12, 263, "blocked", "s")
bar(R, 204, 442); f.text(R + 12, 236, "asleep", "s"); f.text(R + 12, 249, "(a futex wait)", "s")
bar(I, 66, 318); f.text(I + 12, 150, "blocked in", "s"); f.text(I + 12, 163, "InterruptWait()", "s")
bar(I, 434, 560); f.text(I + 12, 480, "InterruptWait()", "s"); f.text(I + 12, 493, "again: unmasks", "s"); f.text(I + 12, 506, "the interrupt", "s")
# 1 send
y = 80
f.num(P + 22, y - 14, 1, "#2c5d9b"); f.arrow([(P, y), (K - 2, y)], "lnk", "ak"); f.text((P + K) / 2 + 14, y - 6, "MsgSendv()", "s c")
note(K, y + 8, ["copy the request to a", "receiving thread; make it", "ready at the caller's priority"], "wk", 164)
f.arrow([(K + 82, y + 32), (R - 2, y + 32)], "lnk", "ak")
# 2 netd thread sleeps
y = 150
f.num(R + 90, y + 8, 2, "#3e7b34")
note(R, y, ["soreceive(): the socket's", "buffer is empty; sleep"], "g", 156)
f.arrow([(R, 200), (K + 2, 200)], "lng", "ag"); f.text((K + R) / 2, 194, "futex wait", "s c")
# 3 interrupt
y = 290
f.num(D + 20, y - 14, 3, "#aa2222"); f.arrow([(D, y), (K + 2, y)], "lnr", "ar"); f.text(672, y - 6, "interrupt", "s c red")
note(K, y + 8, ["identify the vector; mask it;", "wake its thread; end the", "interrupt; reschedule"], "r", 164)
f.arrow([(K + 82, y + 32), (I - 2, y + 32)], "lnk", "ak")
# 4 driver
y = 350
f.num(I + 88, y + 8, 4, "#3e7b34")
note(I, y, ["the driver takes the packet;", "the stack finds the socket", "and queues the datagram;", "wake the sleeper"], "g", 156)
f.arrow([(I, 430), (K + 2, 430)], "lng", "ag"); f.text((R + I) / 2, 425, "futex wake", "s c")
f.arrow([(K + 2, 446), (R - 2, 446)], "lnk", "ak"); f.text((K + R) / 2, 460, "made ready", "s c")
# 5 reply
y = 468
f.num(R + 90, y + 8, 5, "#3e7b34")
note(R, y, ["soreceive() returns", "the datagram"], "g", 156)
f.arrow([(R, 518), (K + 2, 518)], "lng", "ag"); f.text((K + R) / 2, 512, "MsgReply()", "s c")
note(K, 526, ["copy the data to the", "program's buffer; wake it"], "wk", 164)
f.arrow([(K - 82, 543), (P + 9, 543)], "lnk", "ak")
f.box("g", P - 55, 556, 110, 26, 3); f.text(P, 573, "recv() returns", "s c")
f.save("ch04-recv.svg")

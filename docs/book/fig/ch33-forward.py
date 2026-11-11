from svg import Fig
f = Fig(800, 500)
C, Q, S = 120, 400, 680
f.box("r", C - 90, 20, 180, 34); f.text(C, 41, "QNX client", "b c")
f.box("y", Q - 90, 20, 180, 34); f.text(Q, 41, "a thread of qnxd", "b c")
f.box("g", S - 90, 20, 180, 34); f.text(S, 41, "native server", "b c")
for x in (C, Q, S): f.line([(x, 54), (x, 470)], "life")
def step(n, y, x0, x1, label, cls="ln", m="a", col="#333333", sub=None):
    f.arrow([(x0, y), (x1, y)], cls, m)
    mid = (x0 + x1) / 2
    f.num(mid - len(label) * 3.2 - 14, y - 12, n, col); f.text(mid + 6, y - 8, label, "s c")
    if sub: f.text(mid + 25, y + 14, sub, "s c")
# 1 client sends, routed
step(1, 96, C, Q - 2, "MsgSendv() of fstat() on its descriptor", "lnr", "ar", "#aa2222", "the kernel hands it to qnxd")
f.box("y", C - 50, 104, 100, 300, 3); f.text(C, 240, "REPLY-blocked", "s c"); f.text(C, 254, "on qnxd", "s c")
f.box("w", Q - 75, 124, 150, 46, 3); f.text(Q, 142, "2  MsgReceive(), MsgRead()", "s c"); f.text(Q, 158, "converts the request", "s c")
step(3, 200, Q, S - 2, "MsgForwardv(), on the client's connection", "lnk", "ak", "#2c5d9b")
f.box("y", Q - 50, 208, 100, 122, 3); f.text(Q, 262, "REPLY-blocked", "s c"); f.text(Q, 276, "on the server", "s c")
f.box("g", S - 75, 208, 150, 100, 3); f.text(S, 240, "4  sees the client:", "s c"); f.text(S, 255, "its process, thread,", "s c"); f.text(S, 270, "priority, connection", "s c"); f.text(S, 291, "answers in FreeBSD's form", "s c")
step(5, 328, S, Q + 2, "MsgReply(): struct stat, 224 bytes", "lng", "ag", "#3e7b34")
f.box("w", Q - 75, 340, 150, 46, 3); f.text(Q, 358, "6  converts the answer", "s c"); f.text(Q, 374, "to QNX's struct stat", "s c")
step(7, 412, Q, C + 2, "MsgReply(): 120 bytes", "lnr", "ar", "#aa2222")
f.box("r", C - 50, 420, 100, 30, 3); f.text(C, 439, "runs again", "s c")
f.text(400, 490, "Time runs downwards. The client makes one call and is blocked once; it cannot tell that two processes answered it.", "s c")
f.save("ch33-forward.svg")

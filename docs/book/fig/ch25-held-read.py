from svg import Fig
f = Fig(800, 430)
A, P, B = 120, 400, 680
for x, t, sub in [(A, "sh", "has /dev/ttyp0 open"), (P, "pty", "one thread"), (B, "a terminal program", "has /dev/ptyp0 open")]:
    f.box("u", x - 95, 14, 190, 44); f.text(x, 33, t, "b c"); f.text(x, 49, sub, "s c")
    f.line([(x, 58), (x, 415)], "life")
def msg(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    lo = min(x0, x1); f.num(lo + 24, y - 13, n, col); f.text(lo + 40, y - 9, text, "s")
msg(1, 92, A, P - 2, "read, 1024 bytes")
f.box("wu", P - 105, 100, 210, 44, 3); f.text(P, 118, "no line yet: keeps the receive ID,", "s c"); f.text(P, 133, "returns RESMGR_NOREPLY", "s c")
f.box("y", A - 50, 100, 100, 232, 3); f.text(A, 210, "REPLY-", "s c"); f.text(A, 224, "blocked", "s c")
f.box("g", P - 105, 152, 210, 30, 3); f.text(P, 171, "receives again: free for others", "s c")
msg(2, 216, B, P + 2, 'write "ls" and a newline')
f.box("wu", P - 105, 224, 210, 44, 3); f.text(P, 242, "line discipline: a complete", "s c"); f.text(P, 257, "line is ready for the reader", "s c")
msg(3, 296, P, B - 2, "reply: 3 bytes written", "lng", "ag", "#3e7b34")
msg(4, 332, P, A + 52, "reply to the kept receive ID: the line", "lng", "ag", "#3e7b34")
f.box("g", A - 50, 340, 100, 34, 3); f.text(A, 361, "read() returns", "s c")
f.box("g", P - 105, 346, 210, 30, 3); f.text(P, 365, "receives again", "s c")
f.text(400, 404, "Had a signal reached sh while it waited, the kernel would have sent pty an unblock pulse, and pty would have replied EINTR.", "s c")
f.save("ch25-held-read.svg")

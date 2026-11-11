from svg import Fig
f = Fig(800, 420)
A, T1, T2, B = 90, 300, 510, 710
for x, cls, t, sub in [(A, "u", "program A", "recv(s, ...)"), (T1, "k", "netd, thread 1", ""), (T2, "k", "netd, thread 2", ""), (B, "u", "program B", "send(s2, ...)")]:
    f.box(cls, x - 80, 14, 160, 42); f.text(x, 32, t, "b c"); f.text(x, 48, sub, "ms c" if sub else "s c")
    f.line([(x, 56), (x, 406)], "life")
f.text(T1, 48, "waiting to receive", "s c"); f.text(T2, 48, "does not exist yet", "s c")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    f.num(min(x0, x1) + 20, y - 13, n, col); f.text(min(x0, x1) + 36, y - 9, text, "s")
step(1, 90, A, T1 - 2, "read request, a socket receive")
f.box("wk", T1 - 90, 100, 180, 44, 3); f.text(T1, 118, "nobody else is receiving:", "s c"); f.text(T1, 134, "start another thread first", "s c")
f.arrow([(T1 + 90, 122), (T2 - 2, 122)], "ln", "a", dash=True); f.num(T1 + 112, 109, 2)
f.box("wk", T2 - 80, 132, 160, 28, 3); f.text(T2, 150, "waiting to receive", "s c")
f.box("y", T1 - 90, 158, 180, 58, 3); f.text(T1, 176, "soreceive(): the socket's", "s c"); f.text(T1, 191, "buffer is empty,", "s c"); f.text(T1, 206, "the thread sleeps", "s c")
f.box("y", A - 70, 110, 140, 44, 3); f.text(A, 128, "REPLY-blocked", "s c"); f.text(A, 144, "for as long as it takes", "s c")
step(3, 244, B, T2 + 2, "write request, a socket send")
f.box("wk", T2 - 90, 254, 180, 58, 3); f.text(T2, 272, "sosend(): the datagram goes", "s c"); f.text(T2, 287, "down and up the stack to", "s c"); f.text(T2, 302, "A's socket; wake its sleeper", "s c")
f.arrow([(T2 - 90, 290), (T1 + 2, 290)], "lng", "ag"); f.num(T1 + 22, 277, 4, "#3e7b34")
step(5, 340, T2, B - 2, "sent: n bytes", "lng", "ag", "#3e7b34")
f.box("wk", T1 - 90, 300, 180, 30, 3); f.text(T1, 319, "soreceive() returns the data", "s c")
step(6, 372, T1, A + 2, "the data", "lng", "ag", "#3e7b34")
f.text(T1 + 100, 398, "both threads go back to waiting for a message", "s c")
f.save("ch28-threads.svg")

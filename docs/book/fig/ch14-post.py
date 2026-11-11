from svg import Fig
f = Fig(800, 520)
f.text(20, 24, "Posting a signal to a process", "h")
CX = 250
def step(y, n, title, sub, cls="k", h=46):
    f.box(cls, CX - 170, y, 340, h); f.num(CX - 150, y + h / 2, n, "#2c5d9b")
    f.text(CX + 10, y + 19, title, "c"); f.text(CX + 10, y + 35, sub, "s c")
def side(y, text, sub, cls="n"):
    f.arrow([(CX + 170, y + 23), (498, y + 23)])
    f.box(cls, 500, y, 280, 46); f.text(640, y + 19, text, "c"); f.text(640, y + 35, sub, "s c")
Y = [44, 120, 196, 272]
step(Y[0], 1, "SIGSTOP or SIGCONT?", "job control is done first")
side(Y[0], "stop the process / let it go on", "SIGSTOP ends here; SIGCONT goes on down", "r")
step(Y[1], 2, "ignored, and no thread blocks or waits for it?", "action SIG_IGN, or SIG_DFL with a default of nothing")
side(Y[1], "dropped", "nothing is recorded")
step(Y[2], 3, "a stop signal at its default action?", "SIGTSTP, SIGTTIN, SIGTTOU, that no thread blocks")
side(Y[2], "stop the process", "its parent gets SIGCHLD", "r")
step(Y[3], 4, "record it as pending", "for the process, or for the one thread named", "y")
for a, b in zip(Y, Y[1:]):
    f.arrow([(CX, a + 46), (CX, b - 2)])
for y in Y[:3]:
    f.text(CX + 178, y + 16, "yes", "s"); f.text(CX + 8, y + 62, "no", "s")
f.text(500, Y[3] + 19, "real-time signals: every instance is queued", "s"); f.text(500, Y[3] + 35, "others: a second one while pending is dropped", "s")
# who takes it
f.arrow([(CX, Y[3] + 46), (CX, 338), (110, 338), (110, 358)])
f.text(CX + 8, 334, "then look for a thread that can take it", "s")
B = 360
opts = [("g", "a thread waiting for it", "in sigwaitinfo(): woken", "returns the signal's number"),
        ("g", "a thread blocked in a call", "that does not block the signal:", "its call fails with EINTR"),
        ("g", "a thread running user code", "on another CPU: interrupted", "by an interprocessor interrupt"),
        ("n", "none of these", "the signal waits for a thread", "to unblock it")]
for i, (c, a, b, d) in enumerate(opts):
    x = 20 + i * 194
    f.box(c, x, B, 180, 70); f.text(x + 90, B + 20, a, "c"); f.text(x + 90, B + 38, b, "s c"); f.text(x + 90, B + 54, d, "s c")
    if i < 3:
        f.arrow([(x + 180, B + 35), (x + 193, B + 35)], "ln", "a", True)
f.text(400, 462, "Except in the first case the signal stays pending: it is taken, and its action carried out, by whichever thread", "s c")
f.text(400, 478, "next passes through the kernel's return to user mode with the signal unblocked.", "s c")
f.text(400, 502, "The dashed arrows mean \"if there is no such thread\".", "s c")
f.save("ch14-post.svg")

from svg import Fig
f = Fig(800, 300)
f.text(20, 58, "QNX's", "b"); f.text(20, 73, "numbers", "s")
f.text(20, 218, "the kernel's", "b"); f.text(20, 233, "(FreeBSD's)", "s")
X0, W = 110, 5.2
def band(y, a, b, cls, label, sub=None):
    x = X0 + (a - 1) * W; w = (b - a + 1) * W
    f.box(cls, x, y, w, 40, 2)
    f.text(x + w / 2, y + (18 if sub else 24), label, "s c")
    if sub: f.text(x + w / 2, y + 32, sub, "s c")
    return x + w / 2
# QNX row: 1..64
q1 = band(40, 1, 15, "g", "1-15", "the same")
q2 = band(40, 16, 40, "y", "16-40", "other numbers")
q3 = band(40, 41, 56, "u", "41-56", "real-time")
q4 = band(40, 57, 64, "r", "57-64", "QNX's")
# kernel row: 1..126
k1 = band(200, 1, 15, "g", "1-15")
k2 = band(200, 16, 33, "y", "16-33", "FreeBSD's")
k3 = band(200, 34, 56, "w", "34-56", "7 are QNX's")
k4 = band(200, 57, 64, "r", "57-64")
k5 = band(200, 65, 80, "u", "65-80")
k6 = band(200, 81, 126, "w", "81-126", "FreeBSD's other real-time signals")
f.arrow([(q1, 80), (k1, 198)], "lng", "ag", False, True)
f.arrow([(q2 - 20, 80), (k2, 198)], "ln", "a", False, True)
f.arrow([(q2 + 20, 80), (k3 - 10, 198)], "ln", "a", True, True)
f.arrow([(q3, 80), (k5, 198)], "ln", "a", False, True)
f.arrow([(q4, 80), (k4, 198)], "lnr", "ar", False, True)
f.text(620, 96, "16-40, by table:", "s"); f.text(620, 111, "SIGUSR1 16 = 30", "ms"); f.text(620, 124, "SIGCHLD 18 = 20", "ms")
f.text(620, 148, "41-56, by offset:", "s"); f.text(620, 163, "41 = 65 ... 56 = 80", "ms")
f.text(400, 272, "Each QNX number has one kernel number. A kernel number above 80, or one QNX has no name for, is 0 to a QNX process.", "s c")
f.save("ch32-signals.svg")

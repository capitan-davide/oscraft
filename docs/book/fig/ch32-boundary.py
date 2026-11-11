from svg import Fig
f = Fig(800, 400)
f.box("r", 20, 34, 230, 330); f.text(135, 54, "a QNX process", "h c"); f.text(135, 70, "QNX's numbers and layouts", "s c")
f.box("k", 550, 34, 230, 330); f.text(665, 54, "the kernel and process 1", "h c"); f.text(665, 70, "FreeBSD's numbers and layouts", "s c")
f.box("y", 320, 34, 160, 330); f.text(400, 54, "the boundary", "h c"); f.text(400, 70, "conversions", "s c")
rows = [
    ("a call fails", "error 260", "errno_out()", "ETIMEDOUT, 60", "out"),
    ("MsgError(rcvid, 11)", "error 11", "errno_in()", "EAGAIN, 35", "in"),
    ("kill(pid, SIGUSR1)", "signal 16", "signo_in()", "signal 30", "in"),
    ("a handler runs", "signal 18", "signo_out()", "SIGCHLD, 20", "out"),
    ("sigprocmask()", "64-bit set", "set_in(), set_out()", "128-bit set", "both"),
    ("stat()", "120 bytes", "stat_out()", "struct stat, 224 bytes", "out"),
    ("tcgetattr() on the console", "76 bytes", "termios_out()", "struct termios, 44 bytes", "out"),
]
y = 92
for what, q, fn, k, d in rows:
    f.box("w", 30, y, 210, 32, 2); f.text(38, y + 13, what, "s"); f.text(38, y + 27, q, "ms")
    f.box("w", 330, y, 140, 32, 2); f.text(400, y + 20, fn, "ms c")
    f.box("w", 560, y, 210, 32, 2); f.text(665, y + 20, k, "ms c")
    if d == "in":
        f.arrow([(240, y + 16), (328, y + 16)], "lnr", "ar"); f.arrow([(470, y + 16), (558, y + 16)], "lnk", "ak")
    elif d == "out":
        f.arrow([(558, y + 16), (472, y + 16)], "lnk", "ak"); f.arrow([(328, y + 16), (242, y + 16)], "lnr", "ar")
    else:
        f.arrow([(242, y + 16), (328, y + 16)], "ln", "a", False, True); f.arrow([(472, y + 16), (558, y + 16)], "ln", "a", False, True)
    y += 38
f.text(400, 388, "A native process crosses the same boundary with nothing converted.", "s c")
f.save("ch32-boundary.svg")

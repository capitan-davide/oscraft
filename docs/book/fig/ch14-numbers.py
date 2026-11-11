from svg import Fig
f = Fig(800, 300)
f.text(20, 26, "Signal numbers, and one bit for each in a 128-bit set", "h")
X0, W = 30, 740
def x(n): return X0 + (n - 1) * W / 127
Y = 70
segs = [(1, 34, "g", "1 to 33", "FreeBSD's signals"), (34, 57, "n", "34 to 56", "unused by FreeBSD"), (57, 65, "k", "57 to 64", ""), (65, 127, "y", "65 to 126", "real-time signals")]
for a, b, c, t1, t2 in segs:
    f.box(c, x(a), Y, x(b) - x(a), 50, 0)
    if t2:
        f.text((x(a) + x(b)) / 2, Y + 22, t1, "b c"); f.text((x(a) + x(b)) / 2, Y + 39, t2, "s c")
for n in (1, 34, 57, 65, 127):
    f.line([(x(n), Y + 50), (x(n), Y + 58)]); 
for n, lab in ((1, "1"), (34, "34"), (57, "57"), (65, "65"), (127, "126")):
    f.text(x(n) + (0 if n != 127 else -4), Y + 72, lab, "s c")
# notes
f.arrow([(x(17), Y + 120), (x(17), Y + 52)], "lng", "ag")
f.lines(x(1), Y + 136, ["SIGHUP 1 ... SIGKILL 9 ... SIGSEGV 11 ... SIGSTOP 17 ...", "SIGCHLD 20 ... SIGUSR2 31, SIGTHR 32, SIGLIBRT 33.", "One instance pending at a time."], "s", 15)
f.arrow([(x(61), Y - 26), (x(61), Y - 2)], "lnk", "ak")
f.text(x(61), Y - 32, "57 to 64: the system's own; always blocked, queued", "s c blue")
f.arrow([(x(96), Y + 120), (x(96), Y + 52)])
f.lines(x(70), Y + 136, ["SIGRTMIN 65 to SIGRTMAX 126.", "Every instance sent is queued, with its value."], "s", 15)
f.text(400, 286, "SIGKILL and SIGSTOP can be neither blocked, caught nor ignored.", "s c")
f.save("ch14-numbers.svg")

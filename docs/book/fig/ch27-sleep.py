from svg import Fig
f = Fig(800, 420)
cols = [("thread A: waits for data", 20, "u"), ("FreeBSD's code, not edited", 215, "g"), ("bsdkern", 410, "k"), ("thread B: has the data", 605, "u")]
for t, x, cls in cols:
    f.box(cls, x, 14, 175, 30); f.text(x + 87, 34, t, "b c"); f.line([(x + 87, 44), (x + 87, 356)], "life")
def step(col, y, t, s=None, cls="w"):
    x = cols[col][1]; h = 38 if s else 26
    f.box(cls, x + 4, y, 167, h, 3); f.text(x + 87, y + 16, t, "ms c")
    if s: f.text(x + 87, y + 31, s, "s c")
step(0, 58, "msleep(chan, &mtx, ...)")
f.arrow([(191, 71), (217, 71)]); f.num(204, 56, 1)
step(1, 58, "sleepq_add(), sleepq_wait()", "A joins chan's queue")
f.arrow([(302, 96), (302, 106)])
step(1, 108, "mi_switch()", "would switch to another thread")
f.arrow([(386, 127), (412, 127)]); f.num(399, 112, 2)
step(2, 108, "sched_switch()", "set inhibited, drop the lock")
f.arrow([(497, 146), (497, 156)])
step(2, 158, "park", "wait on A's condition variable", "y")
f.box("y", 24, 96, 167, 206, 3); f.text(107, 196, "A's pthread is blocked", "s c"); f.text(107, 212, "in the microkernel", "s c")
step(3, 212, "wakeup(chan)")
f.arrow([(609, 225), (388, 225)]); f.num(497, 211, 3)
step(1, 212, "sleepq_broadcast()", "A leaves chan's queue")
f.arrow([(302, 250), (302, 276), (412, 276)]); f.num(399, 261, 4)
step(2, 264, "sched_wakeup()", "clear inhibited, unpark A")
f.arrow([(414, 294), (193, 294)], "lng", "ag")
step(3, 264, "wakeup() returns", "B never blocked")
step(0, 312, "msleep() returns", "with mtx held again", "g")
notes = ["The sleep queues, their hash chains and their timeouts are FreeBSD's. Only the last step differs:",
         "where a kernel would switch to another thread, bsdkern blocks the pthread, and the microkernel",
         "chooses what runs next. A wakeup that arrives before the park is not lost: the park returns at once."]
f.lines(400, 376, notes, "s c", 16)
f.save("ch27-sleep.svg")

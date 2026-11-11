from svg import Fig
f = Fig(800, 512)
cols = [("u", 95, "program + libthr"), ("g", 285, "libsys"), ("k", 495, "kernel"), ("y", 700, "CPU's timeout queue")]
for cls, x, name in cols:
    f.box(cls, x - 85, 14, 170, 32); f.text(x, 35, name, "c")
    f.line([(x, 46), (x, 462)], "life")
P, L, K, Q = 95, 285, 495, 700
def msg(y, a, b, text, cls="ln", m="a", tcls="s c", dy=-6):
    f.arrow([(a, y), (b + (-2 if b > a else 2), y)], cls, m); f.text((a + b) / 2, y + dy, text, tcls)
msg(90, P, L, "_umtx_op(lock, deadline)")
f.text(P - 80, 66, "pthread_mutex_timedlock()", "s")
msg(112, L, K, "TimerTimeout(clock, SEM | ABSTIME)"); f.num(L - 16, 112, 1)
f.box("wk", K - 95, 120, 190, 38); f.text(K, 136, "thread.armed_timeout =", "s c"); f.text(K, 150, "(deadline, SEM)", "ms c")
msg(172, K, L, "returns at once")
msg(206, L, K, "futex wait on the lock word"); f.num(L - 16, 206, 2)
f.box("wk", K - 95, 214, 190, 38); f.text(K, 230, "still locked: the thread will", "s c"); f.text(K, 244, "block in state SEM", "s c")
msg(270, K, Q, "timeout::arm(SEM)"); f.num((K + Q) / 2 - 70, 264, 3)
f.box("y", Q - 75, 278, 150, 34, 3); f.text(Q, 299, "(deadline, thread)", "ms c")
f.box("y", K - 60, 290, 120, 62, 3); f.text(K, 316, "blocked", "c"); f.text(K, 332, "the mutex stays held", "s c")
f.text(Q, 338, "a tick at or after the deadline", "s c")
msg(384, Q, K, "expire(): unblock(thread, ETIMEDOUT)", "lnr", "ar", "s c red"); f.num(Q + 16, 384, 4, "#aa2222")
msg(416, K, L, "ETIMEDOUT", "lnr", "ar", "s c red")
msg(444, L, P, "ETIMEDOUT", "lnr", "ar", "s c red")
f.text(400, 484, "The deadline is given in one kernel call and used by the next. Had the lock been free, or been released in time,", "s c")
f.text(400, 500, "the armed timeout would simply have been forgotten when the second call returned.", "s c")
f.save("ch13-timed-wait.svg")

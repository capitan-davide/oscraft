from svg import Fig
f = Fig(800, 440)
f.text(400, 22, "a thread is blocked in a kernel call, and something wants it out", "h c")
src = [(40, "a signal arrives for it", "EINTR"), (300, "its timeout runs out", "ETIMEDOUT"), (560, "it must end (its process exits,", "EINTR")]
for x, t, e in src:
    f.box("u", x, 36, 200, 44); f.text(x + 100, 55, t, "c"); f.text(x + 100, 71, e if x != 560 else "another thread destroys it): EINTR", "s c")
    f.arrow([(x + 100, 80), (x + 100, 112)])
f.box("k", 40, 112, 720, 34); f.text(400, 133, "ker::unblock(thread, error): the thread's blocked state says who must release it", "c")
own = [(40, 150, ["Send, Receive,", "Reply"], "msg"), (198, 150, ["Mutex, Condvar,", "Sem, Barrier"], "sync"), (356, 110, ["MqSend,", "MqReceive"], "mqueue"), (474, 80, ["Pipe", ""], "kpipe"), (562, 198, ["Nanosleep, Sigsuspend,", "Join, Intr, ..."], "no queue: sched::wake_with()")]
for x, w, st, o in own:
    f.arrow([(x + w // 2, 146), (x + w // 2, 172)])
    f.box("y", x, 172, w, 62, 3)
    f.text(x + w // 2, 190, st[0], "s c"); f.text(x + w // 2, 203, st[1], "s c")
    f.text(x + w // 2, 224, o, "ms c" if len(o) < 10 else "s c")
# outcomes
f.box("k", 198, 268, 562, 40); f.text(479, 285, "the thread is taken off its queue and made ready;", "c"); f.text(479, 301, "it resumes inside its handler, which returns the error", "s c")
for x in (273, 411, 514, 661):
    f.arrow([(x, 234), (x, 268)])
f.arrow([(140, 234), (140, 252), (240, 252), (240, 268)])
f.box("y", 40, 340, 220, 84, 3)
f.text(150, 360, "REPLY-blocked on a channel", "s c"); f.text(150, 374, "with the unblock flag:", "s c"); f.text(150, 388, "stays blocked; the server gets a", "s c"); f.text(150, 402, "pulse and replies when it is ready", "s c")
f.arrow([(80, 234), (80, 340)], "ln", "a", True)
f.box("r", 290, 340, 220, 84, 3)
f.text(400, 366, "the call fails", "c"); f.text(400, 384, "the error number, at the", "s c"); f.text(400, 398, "failure return point", "s c")
f.box("g", 540, 340, 220, 84, 3)
f.text(650, 358, "the call is made again", "c"); f.text(650, 376, "a native program's MsgSendv() or", "s c"); f.text(650, 390, "futex wait, interrupted for a handler", "s c"); f.text(650, 404, "with SA_RESTART: elr -= 4", "s c")
f.arrow([(400, 308), (400, 340)]); f.arrow([(650, 308), (650, 340)])
f.save("ch07-leaving-early.svg")

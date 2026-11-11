from svg import Fig
f = Fig(800, 420)
f.text(110, 22, "Who delivers", "h c")
src = [("a timer expires", "Chapter 13"), ("an interrupt arrives", "Chapter 15"), ("a server calls", "MsgDeliverEvent()"),
       ("a kernel pipe becomes", "readable or writable"), ("process 1 has news", "a process ended, ...")]
ys = []
for i, (a, b) in enumerate(src):
    y = 36 + i * 62
    f.box("w", 20, y, 180, 46); f.text(110, y + 20, a, "c"); f.text(110, y + 36, b, "s c"); ys.append(y + 23)
f.box("k", 270, 150, 170, 80); f.text(355, 178, "deliver_from()", "m c"); f.text(355, 198, "reads the event's kind", "s c"); f.text(355, 212, "and carries it out", "s c")
for y in ys:
    f.arrow([(200, y), (268, 170 + (y - 59) * 40 // 248)], "ln", "a")
f.text(640, 22, "What the event asked for", "h c")
out = [("y", "a pulse", "queued on a channel, or handed to a receiver"),
       ("y", "a signal", "to the process, or to one of its threads"),
       ("y", "an interrupt wake-up", "InterruptWait() returns in the event's thread"),
       ("y", "a post", "to a named semaphore"),
       ("y", "a memory operation", "set, add or flip bits in one word, atomically"),
       ("y", "a new thread", "that runs the event's function")]
oy = []
for i, (cls, a, b) in enumerate(out):
    y = 36 + i * 50
    f.box(cls, 510, y, 270, 40); f.text(645, y + 17, a, "c"); f.text(645, y + 32, b, "s c"); oy.append(y + 20)
for i, y in enumerate(oy[:4]):
    f.arrow([(440, 160 + i * 14), (508, y)], "lnk", "ak")
# deferred
f.box("k", 300, 300, 170, 60); f.text(385, 322, "a kernel thread", "c"); f.text(385, 338, "takes requests from", "s c"); f.text(385, 351, "a queue; it may block", "s c")
f.arrow([(370, 230), (370, 298)], "lnk", "ak", True)
f.text(362, 262, "queued:", "s e"); f.text(362, 275, "never done in", "s e"); f.text(362, 288, "the deliverer", "s e")
f.arrow([(470, 330), (490, 330), (490, oy[5]), (508, oy[5])], "lnk", "ak")
f.arrow([(490, oy[5]), (490, oy[4] + 8), (508, oy[4] + 8)], "lnk", "ak")
f.arrow([(440, 216), (508, oy[4] - 8)], "lnk", "ak")
f.text(498, 352, "always, for a new thread;", "s"); f.text(498, 366, "for memory, when the deliverer", "s"); f.text(498, 380, "is an interrupt handler", "s")
f.text(400, 408, "Everything above the kernel thread only queues something or wakes a thread, so it is safe inside an interrupt handler.", "s c")
f.save("ch11-delivery.svg")

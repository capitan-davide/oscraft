from svg import Fig
f = Fig(800, 370)
f.text(20, 26, "threads of the server (the microkernel schedules them on the real CPUs)", "h")
ths = [("a client's request", "in a critical section", "g", 130, False), ("the clock", "in a critical section", "g", 310, False),
       ("an interrupt thread", "preempted inside one", "g", 490, False), ("a client's request", "in a critical section", "g", 650, False),
       ("a taskqueue", "waits for a slot", "y", 700, True)]
for i, (t, s, cls, to, wait) in enumerate(ths):
    x = 20 + i * 156
    f.box(cls, x, 40, 136, 74); f.text(x + 68, 58, t, "b c"); f.text(x + 68, 74, s, "s c")
    f.box("w", x + 10, 82, 116, 24, 3); f.text(x + 68, 98, "curthread, curpcpu", "ms c")
    if wait: f.arrow([(x + 68, 114), (to, 202)], "lnr", "ar", True)
    else: f.arrow([(x + 68, 114), (to, 202)], "lng", "ag")
f.text(780, 192, "slots: as many as CPUs", "h e")
for i in range(4):
    x = 60 + i * 180
    f.box("k", x, 204, 140, 64); f.text(x + 70, 222, f"slot {i}", "b c")
    f.box("wk", x + 8, 230, 76, 30, 3); f.text(x + 46, 249, "struct pcpu", "s c")
    f.box("y", x + 90, 230, 42, 30, 3); f.text(x + 111, 249, "mutex", "s c")
notes = ["curthread and curpcpu are thread-local: each thread has its own struct thread, and curpcpu is the slot it holds.",
         "critical_enter(): lock the slot used last time; if it is busy, any free slot; if none is free, wait for the first.",
         "critical_exit(): unlock it. A thread may be preempted while it holds a slot: nobody else can take that slot meanwhile.",
         "A spin mutex is a mutex taken inside a critical section, so a slot always comes before a spin lock."]
f.lines(20, 296, notes, "s", 17)
f.save("ch27-slots.svg")

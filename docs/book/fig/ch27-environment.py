from svg import Fig
f = Fig(800, 400)
needs = [("the current thread and CPU", "mutexes, reader-writer locks"), ("sleep and wakeup", "memory allocators"),
         ("timers and the clock", "start-up order"), ("buses and their resources", "interrupts, addresses for DMA")]
def panel(x, title, sub_note, below, below_cls, below_note, host):
    f.text(x + 180, 30, title, "h c")
    f.box("g", x, 44, 360, 50); f.text(x + 180, 65, "a kernel subsystem", "b c"); f.text(x + 180, 82, sub_note, "s c")
    f.box("y", x, 118, 360, 96); f.text(x + 180, 136, "what it calls and assumes", "b c")
    for i, (a, b) in enumerate(needs):
        f.text(x + 14, 156 + i * 15, a, "s"); f.text(x + 190, 156 + i * 15, b, "s")
    f.box(below_cls, x, 238, 360, 50); f.text(x + 180, 259, below, "b c"); f.text(x + 180, 276, below_note, "s c")
    f.box("n", x, 312, 360, 50); f.text(x + 180, 333, host[0], "b c"); f.text(x + 180, 350, host[1], "s c")
    for y in (94, 214, 288): f.arrow([(x + 180, y), (x + 180, y + 22)])
panel(20, "in its own kernel", "the network stack, a bus, a driver", "the rest of the kernel", "g",
      "scheduler, virtual memory, clock, bus framework", ("the machine", "kernel mode: every instruction is allowed"))
panel(420, "moved into a process", "the same source, not edited", "an environment layer", "k",
      "the same functions, written again over the host", ("the host system", "threads, mutexes, mappings, timers, messages"))
f.line([(400, 20), (400, 372)], "life")
f.text(400, 390, "The interfaces in the middle stay the same. What answers them changes.", "s c")
f.save("ch27-environment.svg")

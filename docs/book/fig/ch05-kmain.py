from svg import Fig
f = Fig(800, 470)
steps = [
 ("this CPU's data; the system page; the console", "from here on the kernel can print", "OsCraft kernel 0.1.0 on CPU 0"),
 ("exception vectors, tested with a breakpoint", "a fault is now reported, not silent", "exceptions: ... self-test ok"),
 ("free memory; startup's page tables; all RAM mapped", "the heap works: Box, Vec, Arc", "pmem: 1844 MiB free in 3 ranges"),
 ("process 1 and its address space; copy windows", "there is a process to own kernel threads", None),
 ("interrupt dispatch, from the system page", "handlers can be attached", None),
 ("the clock tick", "time passes; timeouts work", "clock: 1000 Hz tick on vector 27"),
 ("scheduler, FP and IPIs on this CPU", "threads can be created and switched", None),
 ("the other CPUs, one at a time", "each repeats the per-CPU steps", "smp: 4 of 4 CPUs online"),
 ("interrupts unmasked; process 1's threads created", "the first tick; the boot script is ready", None),
 ("sched::idle(): the boot code becomes the idle thread", "the scheduler picks the first thread", "kernel: running the boot script"),
]
f.text(30, 24, "kmain(), in order", "h"); f.text(362, 24, "what works afterwards", "h"); f.text(584, 24, "on the console", "h")
y = 36
for i, (a, b, log) in enumerate(steps):
    cls = "k" if i < 9 else "g"
    f.box(cls, 20, y, 330, 34, 3); f.num(36, y + 17, i + 1, "#2c5d9b" if i < 9 else "#3e7b34"); f.text(54, y + 21, a, "s")
    f.text(362, y + 21, b, "s")
    if log: f.box("w", 584, y + 5, 206, 24, 2); f.text(590, y + 21, log, "ms")
    if i < 9: f.arrow([(185, y + 34), (185, y + 42)], "ln", "a")
    y += 43
f.save("ch05-kmain.svg")

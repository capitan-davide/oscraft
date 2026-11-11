from svg import Fig
f = Fig(800, 430)
def layer(cls, x, y, w, h, title, lines):
    f.box(cls, x, y, w, h)
    f.text(x + w / 2, y + 22, title, "h c")
    for i, t in enumerate(lines):
        f.text(x + w / 2, y + 40 + i * 14, t, "s c")
layer("k", 20, 15, 630, 66, "proc/   process 1: the managers and built-in servers, as kernel threads",
      ["process, memory and pathname managers; spawn and the ELF loader;", "the image file system, /proc, the consoles; the boot script"])
layer("k", 20, 95, 470, 66, "ker/   the kernel calls",
      ["channels, messages, events, synchronization, signals, threads,", "timers, clocks, interrupts, pipes, message queues; processes; user memory"])
layer("k", 500, 95, 150, 66, "personality/", ["what differs between", "native and QNX processes"])
layer("k", 20, 175, 400, 80, "the core (files directly in kernel/src)",
      ["sched, sporadic: threads and the scheduler", "clock, timeout: the tick and deadlines", "interrupt, ipi, smp, percpu, sync, console, syspage"])
layer("k", 430, 175, 220, 80, "vm/   memory",
      ["pmem: frames;  kmem: kernel pages", "heap: the allocator", "map, object: mappings, objects"])
layer("k", 20, 269, 630, 52, "arch/aarch64/   the processor",
      ["exception vectors, kernel entry and exit, context switch, MMU, user address spaces, timer, interrupt code"])
f.box("y", 20, 345, 630, 40); f.text(335, 362, "system page and callouts", "h c"); f.text(335, 377, "the machine, described by the startup; the only place the kernel learns about it", "s c")
# the use arrow
f.arrow([(700, 30), (700, 315)], "ln", "a")
f.text(712, 160, "each layer", "s"); f.text(712, 174, "uses the", "s"); f.text(712, 188, "ones below", "s")
f.line([(660, 333), (790, 333)], "life"); f.line([(10, 333), (20, 333)], "life")
f.text(712, 358, "data and code", "s"); f.text(712, 372, "from outside", "s"); f.text(712, 386, "the kernel", "s")
f.text(335, 412, "One Rust crate, one executable: the microkernel (the four lower layers) and process 1 (the top one).", "s c")
f.save("ch04-layers.svg")

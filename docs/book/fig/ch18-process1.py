from svg import Fig
f = Fig(800, 400)
# user processes
for i, (n, x) in enumerate([("sh", 40), ("netd", 300), ("ps", 560)]):
    f.box("u", x, 14, 200, 62); f.text(x + 100, 32, n, "b c")
    f.box("wu", x + 10, 42, 180, 26, 3); f.text(x + 100, 59, "connection 0x40000000", "ms c")
    f.arrow([(x + 100, 68), (x + 100, 100), (400 + (i - 1) * 14, 100), (400 + (i - 1) * 14, 150)], "ln", "a")
f.text(408, 94, "every process: fork, wait, mmap, open, sysctl ...", "s")
f.line([(20, 118), (780, 118)], "life"); f.text(776, 114, "user mode", "s e"); f.text(776, 132, "kernel mode", "s e")
# process 1
f.box("k", 20, 138, 760, 248); f.text(34, 158, "process 1 (kernel)", "h")
f.box("y", 300, 152, 200, 26, 3); f.text(400, 169, "channel 1: system manager", "s c")
f.arrow([(400, 178), (400, 198)], "lnk", "ak")
f.box("g", 300, 200, 200, 58, 3); f.text(400, 218, "thread sysmgr", "b c"); f.lines(400, 234, ["receives, dispatches by type,", "replies or holds; never sends"], "s c", 13)
# managers
mg = [("process manager", "fork, wait, IDs, sessions"), ("memory manager", "mmap, shared memory"), ("pathname manager", "who serves a path"), ("built-in files", "/proc, /dev/shmem, image")]
for i, (t, s) in enumerate(mg):
    x = 34 + i * 186
    f.box("wk", x, 290, 176, 40, 3); f.text(x + 88, 306, t, "b c"); f.text(x + 88, 321, s, "s c")
    f.arrow([(400, 258), (400, 274), (x + 88, 274), (x + 88, 288)], "lnk", "ak")
# worker
f.box("g", 560, 200, 206, 58, 3); f.text(663, 218, "thread kernel-worker", "b c"); f.lines(663, 234, ["jobs that must send to", "other servers"], "s c", 13)
f.arrow([(500, 229), (558, 229)], "lnk", "ak", True); f.text(529, 222, "defer", "s c")
# console
f.box("y", 34, 172, 200, 24, 3); f.text(134, 188, "channel 2: console", "s c")
f.box("g", 34, 200, 200, 58, 3); f.text(134, 218, "devtext, devtext-poll", "b c"); f.lines(134, 234, ["the console /dev/text", "on the serial port"], "s c", 13)
f.text(400, 352, "also: bootscript (runs the boot script), sigev-thread (events that start threads),", "s c")
f.text(400, 366, "and the kernel's own threads: one idle thread per processor", "s c")
f.save("ch18-process1.svg")

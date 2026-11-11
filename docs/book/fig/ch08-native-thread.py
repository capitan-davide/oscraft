from svg import Fig
f = Fig(800, 440)
cols = [(100, "the program, libthr", "u"), (300, "libsys", "g"), (500, "the kernel", "k"), (700, "the new thread", "u")]
for x, t, c in cols:
    f.box(c, x - 85, 20, 170, 30); f.text(x, 40, t, "c")
    f.line([(x, 50), (x, 400)], "life")
f.line([(400, 60), (400, 400)], "lnr", True); f.text(396, 72, "EL0", "s e red"); f.text(404, 72, "EL1", "s red")
f.box("wu", 15, 84, 170, 74, 3); f.num(28, 98, 1, "#9a6b2f")
f.text(42, 102, "pthread_create():", "s"); f.text(42, 116, "allocates the stack and", "s"); f.text(42, 130, "the TLS block, fills a", "s"); f.text(42, 144, "struct thr_param", "ms")
f.arrow([(185, 150), (213, 150)]); 
f.box("g", 215, 124, 170, 74, 3); f.num(228, 138, 2, "#3e7b34")
f.text(242, 142, "thr_new(): turns", "s"); f.text(242, 156, "param.rtp (class, prio)", "s"); f.text(242, 170, "into a policy and a", "s"); f.text(242, 184, "kernel priority", "s")
f.arrow([(385, 190), (413, 190)], "lnk", "ak")
f.box("k", 415, 164, 178, 150, 3); f.num(428, 178, 3, "#2c5d9b")
f.text(442, 182, "thr_new call:", "s b")
f.text(424, 200, "takes a thread ID, writes it", "s"); f.text(424, 214, "to child_tid and parent_tid;", "s")
f.text(424, 232, "start(): a kernel stack with", "s"); f.text(424, 246, "a trap frame: pc = start_func,", "s"); f.text(424, 260, "x0 = arg, sp = stack top;", "s"); f.text(424, 274, "thread pointer = tls_base;", "s"); f.text(424, 288, "then the ready queue", "s")
f.arrow([(593, 300), (613, 300)], "lng", "ag")
f.box("g", 615, 280, 170, 62, 3); f.num(628, 294, 4, "#3e7b34")
f.text(642, 298, "first return to EL0:", "s"); f.text(642, 312, "start_func(arg), on its", "s"); f.text(642, 326, "own stack, with its ID", "s")
f.arrow([(500, 350), (187, 350)], "ln", "a", True); f.text(300, 342, "0: created", "s c")
f.box("wu", 15, 332, 170, 44, 3); f.num(28, 346, 5, "#9a6b2f"); f.text(42, 350, "pthread_create() returns;", "s"); f.text(42, 364, "the ID is in the pthread", "s")
f.text(400, 424, "The kernel builds nothing in the new thread's memory except its ID: stack and thread-local storage are the C library's.", "s c")
f.save("ch08-native-thread.svg")

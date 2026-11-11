from svg import Fig
f = Fig(800, 350)
# ps process
f.box("u", 20, 14, 330, 220); f.text(34, 34, "ps", "h")
layers = [("ps", "prints a line per record", "g"), ("libkvm: kvm_getprocs()", "asks for all processes", "g"), ("libc: sysctl()", "name: kern.proc ...", "g"), ("libsys", "a value of its own? no: send to process 1", "k")]
for i, (t, s, c) in enumerate(layers):
    y = 46 + i * 46
    f.box(c, 34, y, 302, 38, 3); f.text(44, y + 16, t, "b"); f.text(44, y + 31, s, "s")
    if i: f.arrow([(185, y - 8), (185, y - 1)])
# process 1
f.box("k", 450, 14, 330, 220); f.text(464, 34, "process 1", "h")
f.box("wk", 464, 46, 302, 38, 3); f.text(474, 62, "sysmgr thread", "b"); f.text(474, 77, "a sysctl message: its own handler", "s")
f.box("wk", 464, 92, 302, 38, 3); f.text(474, 108, "for each entry of the process table", "b"); f.text(474, 123, "fill one struct kinfo_proc", "s")
f.arrow([(615, 84), (615, 91)], "lnk", "ak")
f.box("y", 464, 140, 302, 84, 3); f.text(474, 156, "struct kinfo_proc: 1088 bytes", "b")
f.lines(474, 172, ["ki_pid, ki_ppid, ki_pgid, ki_sid, ki_uid ...", "ki_stat: from the threads' states", "ki_runtime: CPU time of its threads", "ki_comm: the program's name"], "ms", 13)
f.arrow([(615, 130), (615, 139)], "lnk", "ak")
# arrows between
f.arrow([(336, 196), (400, 196), (400, 58), (462, 58)], "lnk", "ak"); f.num(400, 120, 1, "#2c5d9b")
f.arrow([(464, 212), (338, 212)], "lng", "ag"); f.num(400, 224, 2, "#3e7b34")
f.text(400, 250, "one message, one reply:", "s c"); f.text(400, 264, "all the records", "s c")
# output
f.box("w", 20, 284, 760, 54, 3)
for i, row in enumerate([("PID", "TT", "STAT", "TIME", "COMMAND"), ("28681", "text", "Ss+", "0:00.19", "/bin/sh")]):
    y = 304 + i * 18
    f.text(80, y, row[0], "m e"); f.text(124, y, row[1], "m e"); f.text(136, y, row[2], "m"); f.text(236, y, row[3], "m e"); f.text(246, y, row[4], "m")
f.arrow([(120, 234), (120, 282)]); f.num(134, 258, 3)
f.text(770, 316, "the arguments come from a second request, per process", "s e")
f.save("ch18-ps.svg")

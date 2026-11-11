from svg import Fig
f = Fig(800, 340)
# the process
f.box("u", 20, 20, 520, 280); f.text(36, 42, "a process", "h")
# address space
f.box("wu", 36, 56, 190, 228); f.text(131, 74, "address space", "b c")
for i, (t, c) in enumerate([("program code", "k"), ("data", "y"), ("heap", "y"), ("stack of thread 1", "g"), ("stack of thread 2", "g")]):
    f.box(c, 50, 86 + i * 38, 162, 28, 3); f.text(131, 104 + i * 38, t, "s c")
# threads
f.box("wu", 242, 56, 130, 110); f.text(307, 74, "threads", "b c")
for i in range(2):
    f.box("g", 254, 86 + i * 38, 106, 28, 3); f.text(307, 104 + i * 38, f"thread {i+1}: registers", "s c")
f.arrow([(254, 100), (238, 100), (238, 214), (214, 214)], "lng", "ag")
f.arrow([(254, 138), (232, 138), (232, 252), (214, 252)], "lng", "ag")
# identity
f.box("wu", 242, 178, 130, 106); f.text(307, 196, "identity", "b c")
f.lines(307, 216, ["process ID, parent", "user and group IDs", "privileges", "exit status"], "s c", 16)
# handles
f.box("wu", 388, 56, 136, 228); f.text(456, 74, "table of handles", "b c")
hs = ["0", "1", "2", "3", "4"]
for i, h in enumerate(hs):
    f.box("y", 402, 86 + i * 32, 30, 24, 2); f.text(417, 103 + i * 32, h, "m c")
    f.box("w", 432, 86 + i * 32, 78, 24, 2)
f.text(456, 262, "small integers,", "s c"); f.text(456, 275, "private to the process", "s c")
# kernel objects
f.box("k", 600, 56, 180, 228); f.text(690, 74, "objects in the kernel", "b c")
objs = [("terminal", 92), ("open file", 148), ("socket", 204)]
for t, y in objs:
    f.box("wk", 620, y, 140, 34, 3); f.text(690, y + 21, t, "c")
f.arrow([(510, 98), (560, 98), (560, 104), (618, 104)], "lnk", "ak")
f.arrow([(510, 130), (566, 130), (566, 112), (618, 112)], "lnk", "ak")
f.arrow([(510, 162), (618, 162)], "lnk", "ak")
f.arrow([(510, 194), (566, 194), (566, 216), (618, 216)], "lnk", "ak")
f.text(471, 230, "free", "s c")
f.text(400, 326, "Threads share everything in the box. Another process can reach none of it, except through the kernel.", "s c")
f.save("ch17-anatomy.svg")

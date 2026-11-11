from svg import Fig
f = Fig(800, 440)
# process table
f.text(100, 24, "the process table", "h c"); f.text(100, 40, "in slot order", "s c")
rows = [("1", "1", "kernel"), ("2", "2", "pipe"), ("6", "4102", "syslogd"), ("7", "8199", "netd"), ("9", "28681", "sh")]
f.text(36, 62, "slot", "s c"); f.text(84, 62, "ID", "s c"); f.text(146, 62, "program", "s c")
for i, (s, pid, name) in enumerate(rows):
    y = 70 + i * 28
    cls = "y" if name == "sh" else "wk"
    f.box(cls, 20, y, 32, 24, 2); f.text(36, y + 16, s, "m c")
    f.box(cls, 52, y, 64, 24, 2); f.text(84, y + 16, pid, "m c")
    f.box(cls, 116, y, 64, 24, 2); f.text(148, y + 16, name, "s c")
# pid bits
f.box("w", 20, 236, 160, 86); f.text(100, 254, "28681 = 0x7009", "m c")
f.box("k", 34, 264, 76, 22, 2); f.text(72, 279, "0x7000", "m c")
f.box("y", 110, 264, 56, 22, 2); f.text(138, 279, "009", "m c")
f.text(72, 300, "generation", "s c"); f.text(138, 300, "slot", "s c")
f.text(100, 314, "bits 12 and up | bits 0-11", "s c")
# the process
f.box("k", 230, 14, 250, 322); f.text(355, 34, "Process (sh)", "h c")
f.arrow([(180, 194), (228, 194)], "lnk", "ak")
groups = [("identity", "pid, ppid, pgid, sid, cred, personality", 46),
          ("memory", "aspace (page tables), mem (regions)", 84),
          ("signals, limits, abilities", "each under its own lock", 122)]
for t, s, y in groups:
    f.box("wk", 242, y, 226, 32, 3); f.text(250, y + 14, t, "b"); f.text(250, y + 27, s, "s")
def table(y, title, cells):
    f.text(250, y + 10, title, "b")
    for i, c in enumerate(cells):
        cls = "y" if c not in ("", "-") else "w"
        f.box(cls, 250 + i * 36, y + 16, 36, 22, 2); f.text(268 + i * 36, y + 31, c, "ms c")
table(162, "channels[ ]   none: sh serves nobody", ["-", "", "", "", "", ""])
table(214, "connections[ ]   file descriptors", ["0", "1", "2", "3", "", ""])
table(266, "side_connections[ ]   from 0x40000000", ["0", "", "", "", "", ""])
f.text(355, 326, "an entry takes the lowest free index", "s c")
# targets
f.box("k", 560, 14, 220, 78); f.text(670, 32, "process 1", "h c")
f.box("wk", 574, 42, 192, 20, 3); f.text(670, 56, "channel 1: system manager", "s c")
f.box("wk", 574, 66, 192, 20, 3); f.text(670, 80, "channel 2: console", "s c")
f.box("u", 560, 110, 220, 52); f.text(670, 128, "tmpfs", "h c")
f.box("wu", 574, 136, 192, 20, 3); f.text(670, 150, "channel 1: files under /tmp", "s c")
# arrows
f.arrow([(268, 252), (268, 258), (500, 258), (500, 76), (572, 76)], "ln", "a")   # fd 0 -> console
f.arrow([(376, 252), (376, 262), (540, 262), (540, 146), (572, 146)], "ln", "a") # fd 3 -> tmpfs
f.arrow([(268, 304), (268, 312), (490, 312), (490, 52), (572, 52)], "ln", "a")   # side 0 -> sysmgr
f.lines(670, 200, ["A connection holds its channel.", "A channel names its owner by", "process ID only, so it never", "keeps a process alive."], "s c", 15)
# global tables
f.box("n", 20, 356, 760, 68); f.text(34, 374, "kernel-wide tables, entries marked with their process", "b")
items = [("threads", "each holds a counted reference to its Process", 34), ("timers", "removed when the process ends", 300), ("interrupt attachments", "detached when the process ends", 540)]
for t, s, x in items:
    f.box("w", x, 382, 228 if x < 500 else 226, 34, 3); f.text(x + 10, 396, t, "b"); f.text(x + 10, 410, s, "s")
f.arrow([(148, 382), (148, 340), (240, 340), (240, 337)], "lnk", "ak")
f.save("ch17-tables.svg")

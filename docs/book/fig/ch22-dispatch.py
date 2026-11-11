from svg import Fig
f = Fig(820, 420)
# the message
f.box("y", 20, 120, 190, 96, 3)
f.text(115, 140, "a connect message", "b c")
f.text(34, 162, "request: open", "ms")
f.text(34, 178, "handle:  5", "ms")
f.text(34, 194, "path:    self/cmdline", "ms")
f.text(115, 236, "the handle came from the", "s c")
f.text(115, 251, "pathname manager's reply", "s c")
f.arrow([(210, 168), (268, 168)], "lnk", "ak")
# process 1
f.box("k", 270, 16, 530, 390)
f.text(286, 36, "process 1", "b")
f.box("wk", 286, 46, 498, 286, 3)
f.text(300, 66, "channel 1: one thread receives every request", "s")
rows = [("0", "the pathname manager", "Chapter 21"), ("1", "the image file system", "/ and the boot image's files"),
        ("2, 3, 10", "the simple devices", "/dev/null, /dev/zero, /dev/klog"), ("4", "/dev/tty", "a link to the caller's terminal"),
        ("5", "the process file system", "/proc"), ("6, 7", "memory objects", "/dev/shmem, typed memory"),
        ("9", "named semaphores", "/dev/sem"), ("256 ...", "directories of the name space", "/dev, /dev/socket ...")]
f.text(300, 90, "handle", "s"); f.text(380, 90, "goes to", "s")
for i, (h, who, what) in enumerate(rows):
    y = 98 + i * 28
    cls = "g" if h == "5" else "w"
    f.box(cls, 296, y, 478, 23, 3)
    f.text(306, y + 16, h, "m"); f.text(380, y + 16, who, ""); f.text(764, y + 16, what, "s e")
f.box("wk", 286, 342, 498, 52, 3)
f.text(300, 362, "channel 2: a thread of its own", "s")
f.text(300, 381, "the console", ""); f.text(764, 381, "/dev/text, /dev/console", "s e")
f.save("ch22-dispatch.svg")

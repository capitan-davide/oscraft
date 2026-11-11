from svg import Fig
f = Fig(800, 300)
def panel(x0, title):
    f.text(x0 + 190, 24, title, "h c")
    f.box("u", x0 + 20, 38, 120, 28); f.box("y", x0 + 150, 38, 80, 28, 0); f.box("u", x0 + 240, 38, 120, 28)
    f.text(x0 + 80, 56, "writer", "c"); f.text(x0 + 190, 56, "pipe", "s c"); f.text(x0 + 300, 56, "reader", "c")
    f.line([(x0 + 80, 66), (x0 + 80, 200)], "life"); f.line([(x0 + 300, 66), (x0 + 300, 200)], "life")
# left: last writer closes
x0 = 10
panel(x0, "The last writer closes")
f.box("y", x0 + 250, 78, 100, 60, 3); f.lines(x0 + 300, 98, ["read() waits:", "the pipe is", "empty"], "s c", 13)
f.box("w", x0 + 30, 100, 100, 26, 3); f.text(x0 + 80, 117, "close()", "s c")
f.arrow([(x0 + 130, 150), (x0 + 248, 150)], "lnk", "ak", True); f.text(x0 + 190, 144, "writers: 0", "s c")
f.box("g", x0 + 250, 146, 100, 44, 3); f.lines(x0 + 300, 164, ["read() returns 0:", "end of file"], "s c", 13)
f.lines(x0 + 20, 224, ["Data still in the buffer is read first.", "A reader in poll() is told too."], "s", 14)
f.line([(400, 14), (400, 280)], "life")
x0 = 410
panel(x0, "The last reader closes")
f.box("y", x0 + 30, 78, 100, 60, 3); f.lines(x0 + 80, 98, ["write() waits:", "the pipe is", "full"], "s c", 13)
f.box("w", x0 + 250, 100, 100, 26, 3); f.text(x0 + 300, 117, "close()", "s c")
f.arrow([(x0 + 248, 150), (x0 + 142, 150)], "lnr", "ar", True); f.text(x0 + 190, 144, "readers: 0", "s c")
f.box("r", x0 + 20, 146, 120, 44, 3); f.lines(x0 + 80, 164, ["SIGPIPE, and", "write() fails: EPIPE"], "s c", 13)
f.lines(x0 + 20, 224, ["Nobody will ever read what is written, so the", "writer is stopped rather than left to fill the buffer."], "s", 14)
f.text(400, 290, "A pipe counts the descriptors open on each side; an end is gone only when its last copy is closed.", "s c")
f.save("ch12-pipe-ends.svg")

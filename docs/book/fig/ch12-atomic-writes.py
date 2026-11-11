from svg import Fig
f = Fig(800, 250)
f.box("u", 20, 20, 190, 46); f.text(115, 40, "writer A", "c"); f.text(115, 56, "lines of 80 bytes: small", "s c")
f.box("u", 20, 84, 190, 46); f.text(115, 104, "writer B", "c"); f.text(115, 120, "one write of 8000 bytes: large", "s c")
f.arrow([(210, 43), (258, 70)], "ln", "a"); f.arrow([(210, 107), (258, 80)], "ln", "a")
f.text(520, 34, "what the reader reads, in order", "s c b")
segs = [("k", 70, "A: 80"), ("y", 150, "B: part 1"), ("k", 70, "A: 80"), ("y", 130, "B: part 2"), ("k", 70, "A: 80")]
x = 262
for cls, w, t in segs:
    f.box(cls, x, 52, w, 44, 0); f.text(x + w / 2, 79, t, "s c"); x += w
f.text(x + 8, 79, "...", "s")
f.lines(262, 126, ["A small write (at most the pipe's atomic size, PIPE_BUF) goes into the buffer in one",
                   "piece. If there is no room for all of it, the writer waits: A's lines always arrive whole.",
                   "",
                   "A large write goes in as room is freed, a piece at a time. Other writers' data can land",
                   "between the pieces, and the reader cannot tell where B's write was cut.",
                   ], "s", 15)
f.text(400, 232, "The guarantee is what lets several programs write records or log lines to one pipe without a lock.", "s c")
f.save("ch12-atomic-writes.svg")

from svg import Fig
f = Fig(800, 330)
# stream
f.text(200, 24, "A pipe: a stream of bytes", "h c")
f.text(20, 52, "written:", "s")
x = 80
for w, t in [(56, "5"), (90, "9"), (56, "3")]:
    f.box("k", x, 38, w, 24, 0); f.text(x + w / 2, 55, t + " bytes", "s c"); x += w + 8
f.arrow([(200, 66), (200, 88)], "ln", "a")
f.box("y", 80, 90, 218, 26, 0); f.text(189, 107, "17 bytes, in the order written", "s c")
f.arrow([(200, 120), (200, 142)], "ln", "a")
f.text(20, 162, "read:", "s")
x = 80
for w, t in [(96, "8"), (96, "8"), (14, "1")]:
    f.box("g", x, 146, w, 24, 0); f.text(x + w / 2, 163, t, "s c"); x += w + 4
f.lines(20, 204, ["The writes' boundaries are gone: the reader asked for", "8 bytes at a time and got 8, 8 and 1.", "", "One order only: first in, first out.", "The two ends are a writer and a reader; whoever", "holds the descriptors may use them."], "s", 15)
f.line([(400, 14), (400, 316)], "life")
# queue
f.text(600, 24, "A message queue: whole messages", "h c")
f.text(420, 52, "sent:", "s")
sent = [("A", "5"), ("B", "20"), ("C", "5"), ("D", "20")]
x = 470
for n, p in sent:
    f.box("k", x, 38, 70, 24, 0); f.text(x + 35, 55, n + "  prio " + p, "s c"); x += 76
f.arrow([(600, 66), (600, 88)], "ln", "a")
f.text(420, 107, "queue:", "s")
f.box("y", 470, 90, 146, 26, 0); f.text(543, 107, "prio 20:  B  D", "s c")
f.box("y", 622, 90, 146, 26, 0); f.text(695, 107, "prio 5:  A  C", "s c")
f.arrow([(600, 120), (600, 142)], "ln", "a")
f.text(420, 162, "received:", "s")
x = 480
for n in ["B", "D", "A", "C"]:
    f.box("g", x, 146, 64, 24, 0); f.text(x + 32, 163, n, "s c"); x += 70
f.lines(420, 204, ["Each receive returns exactly one message, as sent.", "", "The highest priority first; among equals, the oldest.", "The queue has a name; any process allowed to open", "it may send or receive. Its size is fixed when it is", "made: so many messages of at most so many bytes."], "s", 15)
f.save("ch12-stream-queue.svg")

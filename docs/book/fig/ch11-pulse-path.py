from svg import Fig
f = Fig(800, 360)
# the pulse as received
f.text(150, 26, "A pulse, as the receiver gets it", "h c")
X, Y = 30, 44
cells = [("type 0", 60, "0"), ("subtype 0", 60, "2"), ("code", 40, "4"), ("", 40, "5"), ]
x = X
row = [("type", 56, "0", "w"), ("subtype", 56, "2", "w"), ("code", 44, "4", "y"), ("", 84, "5", "n")]
for name, w, off, cls in row:
    f.box(cls, x, Y, w, 30, 0); f.text(x + w / 2, Y + 19, name, "s c"); f.text(x + 2, Y + 44, off, "ms"); x += w
x = X
row2 = [("value (64 bits)", 240, "8", "y")]
f.box("y", X, Y + 56, 240, 30, 0); f.text(X + 120, Y + 75, "value: 64 bits", "s c"); f.text(X + 2, Y + 100, "8", "ms")
f.box("k", X, Y + 112, 120, 30, 0); f.text(X + 60, Y + 131, "scoid", "s c"); f.text(X + 2, Y + 156, "16", "ms")
f.box("n", X + 120, Y + 112, 120, 30, 0); f.text(X + 122, Y + 156, "20", "ms")
f.lines(X, Y + 186, ["24 bytes. Type and subtype are 0. The code is", "one signed byte: below 0 the kernel's, from 0", "up a program's own. The scoid says which", "client connection the pulse came on.",
                     "", "The priority is not in the bytes: the receiving", "thread runs at it."], "s", 14)
# the path
f.line([(300, 14), (300, 346)], "life")
f.text(555, 26, "Sending one: never a wait", "h c")
f.box("w", 330, 44, 150, 40); f.text(405, 61, "a pulse for the channel", "s c"); f.text(405, 75, "priority 21", "s c")
f.arrow([(480, 64), (538, 64)], "ln", "a")
f.box("k", 540, 44, 240, 40); f.text(660, 61, "is a server thread waiting", "s c"); f.text(660, 75, "to receive on the channel?", "s c")
# yes
f.arrow([(600, 84), (600, 118)], "lng", "ag"); f.text(608, 106, "yes", "s green")
f.box("g", 540, 120, 120, 62); f.lines(600, 138, ["copied into its", "buffer; it wakes", "at priority 21"], "s c", 14)
# no
f.arrow([(720, 84), (720, 118)], "ln", "a"); f.text(728, 106, "no", "s")
f.box("y", 670, 120, 110, 62); f.lines(725, 138, ["kept in the", "channel's", "pulse queue"], "s c", 14)
# the queue
f.text(515, 214, "The channel's pulse queue: by priority, then arrival", "s c b")
q = [("30", "w"), ("21", "w"), ("21", "y"), ("10", "w"), ("10", "w")]
x = 344
for i, (p, cls) in enumerate(q):
    f.box(cls, x, 228, 66, 34, 2); f.text(x + 33, 249, "prio " + p, "s c"); x += 70
f.text(344 + 2 * 70 + 33, 278, "the new one: behind", "s c"); f.text(344 + 2 * 70 + 33, 291, "the earlier 21", "s c")
f.arrow([(725, 182), (725, 245), (692, 245)], "ln", "a")
f.lines(330, 316, ["Queued messages and pulses raise the server's last receiver to their priority, so the", "queue is emptied at the priority of the most urgent thing in it (Chapter 10)."], "s", 14)
f.save("ch11-pulse-path.svg")

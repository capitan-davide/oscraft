from svg import Fig
f = Fig(800, 340)
# left panel: asynchronous
f.text(200, 28, "Asynchronous: the kernel keeps the message", "h c")
f.box("u", 30, 60, 120, 70); f.lines(90, 88, ["sender", "goes on running"], "c"); f.text(90, 118, "", "s c")
f.box("k", 30+135, 50, 70, 180); f.text(200, 68, "kernel", "b c"); f.text(200, 83, "queue", "s c")
for i in range(4):
    f.box("wk", 175, 95 + i*32, 50, 24, 2); f.text(200, 111 + i*32, f"msg {4-i}", "ms c")
f.box("u", 250, 150, 120, 70); f.lines(310, 180, ["receiver", "takes one later"], "c")
f.arrow([(150, 95), (173, 107)], "lnk", "ak"); f.text(160, 86, "copy 1", "s c blue")
f.line([(150, 95), (150, 95)])
f.arrow([(227, 203), (248, 190)], "lnk", "ak"); f.text(240, 222, "copy 2", "s c blue")
f.lines(200, 268, ["Two copies of every message.", "The queue has a size: when it is full,", "the sender blocks after all, or the message is lost.", "A receiver cannot tell how old a message is."], "s c", 16)
# divider
f.line([(400, 20), (400, 325)], "life")
# right panel: synchronous
f.text(600, 28, "Synchronous: the sender waits", "h c")
f.box("u", 430, 60, 150, 110); f.text(505, 80, "sender", "b c"); f.box("wu", 445, 92, 120, 28, 2); f.text(505, 110, "its own buffer", "ms c")
f.box("y", 445, 130, 120, 28, 2); f.text(505, 148, "blocked", "s c")
f.box("u", 630, 60, 150, 110); f.text(705, 80, "receiver", "b c"); f.box("wu", 645, 92, 120, 28, 2); f.text(705, 110, "its own buffer", "ms c")
f.text(705, 148, "works, then replies", "s c")
f.arrow([(565, 106), (643, 106)], "lnk", "ak"); f.text(604, 98, "one copy", "s c blue")
f.arrow([(643, 144), (567, 144)], "lng", "ag"); f.text(604, 162, "reply", "s c green")
f.box("k", 430, 190, 350, 34); f.text(605, 211, "kernel: no message storage, only who waits for whom", "s c")
f.lines(600, 268, ["One copy, straight between the two address spaces.", "Nothing to size, fill or lose.", "The reply tells the sender the work is done.", "The wait is also a scheduling fact: the receiver runs for the sender."], "s c", 16)
f.save("ch10-ipc-models.svg")

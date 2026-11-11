from svg import Fig
f = Fig(800, 430)
cx, sx = 230, 570
f.box("u", cx - 70, 20, 140, 32); f.text(cx, 41, "client", "c")
f.box("u", sx - 70, 20, 140, 32); f.text(sx, 41, "server", "c")
f.line([(cx, 52), (cx, 400)], "life"); f.line([(sx, 52), (sx, 400)], "life")
mid = (cx + sx) // 2
# 1 request
y = 84
f.arrow([(cx, y), (sx - 2, y)], "lnk", "ak"); f.num(cx + 64, y - 14, 1, "#2c5d9b")
f.text(cx + 80, y - 10, "send: \"tell me when there is data\" + an event", "s")
f.box("y", cx - 45, y + 5, 90, 30, 3); f.text(cx, y + 24, "blocked", "s c")
f.box("g", sx - 45, y + 5, 90, 30, 3); f.text(sx, y + 24, "keeps the event", "s c")
# 2 reply
y = 132
f.arrow([(sx, y), (cx + 2, y)], "lng", "ag"); f.num(cx + 64, y - 14, 2, "#3e7b34")
f.text(cx + 80, y - 10, "reply at once: \"nothing yet\"", "s")
f.box("g", cx - 45, y + 8, 90, 96, 3); f.lines(cx, y + 46, ["free: other", "work, or one", "wait for all", "its events"], "s c", 13)
f.box("y", sx - 45, y + 8, 90, 70, 3); f.lines(sx, y + 40, ["waits for", "other clients"], "s c", 13)
# data arrives
y = 226
f.arrow([(760, y), (sx + 47, y)], "ln", "a"); f.text(700, y - 8, "data arrives", "s c")
f.box("g", sx - 45, y - 12, 90, 44, 3); f.lines(sx, y + 6, ["delivers", "the event"], "s c", 13)
# 3 event
y = 262
f.arrow([(sx - 47, y), (cx + 2, y)], "lnk", "ak", True); f.num(cx + 64, y - 14, 3, "#2c5d9b")
f.text(cx + 80, y - 10, "the event: no reply, no waiting", "s")
f.box("y", sx - 45, y + 14, 90, 30, 3); f.text(sx, y + 33, "waits again", "s c")
# 4 read
y = 316
f.box("g", cx - 45, y - 46, 90, 40, 3); f.lines(cx, y - 30, ["notices", "the event"], "s c", 13)
f.arrow([(cx, y), (sx - 2, y)], "lnk", "ak"); f.num(cx + 64, y - 14, 4, "#2c5d9b")
f.text(cx + 80, y - 10, "send: \"read\"", "s")
f.box("y", cx - 45, y + 5, 90, 30, 3); f.text(cx, y + 24, "blocked", "s c")
f.box("g", sx - 45, y + 5, 90, 30, 3); f.text(sx, y + 24, "copies the data", "s c")
y = 364
f.arrow([(sx, y), (cx + 2, y)], "lng", "ag"); f.num(cx + 64, y - 14, 5, "#3e7b34")
f.text(cx + 80, y - 10, "reply: the data", "s")
f.text(400, 420, "Every solid arrow is a synchronous message from client to server or its reply. Only the dashed one goes upwards, and it never blocks.", "s c")
f.save("ch11-notify-sequence.svg")

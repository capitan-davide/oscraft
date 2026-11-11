from svg import Fig
f = Fig(800, 390)
# the name manager and the client, outside the server
f.box("k", 20, 20, 210, 46); f.text(125, 40, "name manager", "b c"); f.text(125, 56, "/dev/random belongs to this server", "s c")
f.box("u", 20, 170, 150, 86); f.text(95, 192, "client", "b c"); f.text(95, 210, "blocked from its", "s c"); f.text(95, 224, "request to the reply", "s c")
# the server
f.box("u", 290, 20, 490, 350); f.text(535, 42, "server process", "b c")
f.box("wu", 310, 160, 120, 48, 3); f.text(370, 181, "receive", "b c"); f.text(370, 197, "wait for a message", "s c")
f.box("wu", 470, 160, 120, 48, 3); f.text(530, 181, "dispatch", "b c"); f.text(530, 197, "by message type", "s c")
# the handler table
f.text(695, 76, "handlers", "s c")
rows = ["open", "read", "write", "close", "..."]
for i, r in enumerate(rows):
    f.box("g" if r == "read" else "w", 640, 84 + i * 28, 110, 28, 0); f.text(695, 102 + i * 28, r, "m c")
f.box("y", 470, 292, 280, 52, 3); f.text(610, 313, "state of each open file", "b c"); f.text(610, 330, "which client, which file, its position", "s c")
f.box("wu", 310, 292, 120, 52, 3); f.text(370, 314, "reply", "b c"); f.text(370, 331, "status and data", "s c")
# 1: register
f.arrow([(290, 43), (232, 43)], "lnk", "ak"); f.num(262, 28, 1, "#2c5d9b")
f.text(282, 84, "claims a name", "s e")
# 2: request arrives
f.arrow([(170, 184), (308, 184)], "lnk", "ak"); f.num(206, 170, 2, "#2c5d9b"); f.text(222, 174, "request", "s")
# 3: receive -> dispatch
f.arrow([(430, 184), (468, 184)]); f.num(449, 168, 3)
# 4: dispatch -> handler row "read"
f.arrow([(590, 184), (615, 184), (615, 126), (638, 126)]); f.num(615, 150, 4)
# 5: handler -> state
f.arrow([(695, 224), (695, 290)], "ln", "a", False, True); f.num(712, 258, 5)
# 6: -> reply
f.arrow([(468, 318), (432, 318)]); 
# 7: reply to the client
f.arrow([(310, 318), (240, 318), (240, 236), (172, 236)], "lng", "ag"); f.num(240, 276, 6, "#3e7b34"); f.text(206, 252, "reply", "s c")
# back to receive
f.arrow([(370, 292), (370, 210)], "ln", "a", True); f.text(378, 256, "and again", "s")
f.save("ch25-server-loop.svg")

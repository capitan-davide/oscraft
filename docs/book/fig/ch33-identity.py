from svg import Fig
f = Fig(800, 330)
f.box("r", 20, 60, 190, 110); f.text(115, 82, "QNX client", "b c"); f.text(115, 102, "process 31, thread 1", "s c"); f.text(115, 118, "priority 40", "s c"); f.text(115, 142, "REPLY-blocked on qnxd", "s c")
f.box("y", 300, 60, 200, 110); f.text(400, 82, "qnxd's thread", "b c"); f.text(400, 102, "took priority 40 when it received", "s c"); f.text(400, 118, "acting for: the client", "s c"); f.text(400, 142, "REPLY-blocked on the server", "s c")
f.box("g", 590, 60, 190, 110); f.text(685, 82, "native server", "b c"); f.text(685, 102, "takes priority 40", "s c"); f.text(685, 118, "from the message", "s c")
f.arrow([(210, 100), (298, 100)], "lnr", "ar"); f.text(254, 92, "send", "s c")
f.arrow([(500, 100), (588, 100)], "lnk", "ak"); f.text(544, 92, "forward", "s c")
f.arrow([(588, 140), (502, 140)], "lng", "ag"); f.text(545, 156, "reply", "s c green")
f.arrow([(298, 140), (212, 140)], "lng", "ag"); f.text(255, 156, "reply", "s c green")
# the info record
f.text(400, 206, "What the server is told about the message", "h c")
cols = [("sender's process", "31", "r"), ("sender's thread", "1", "r"), ("priority", "40", "r"), ("connection IDs", "the client's", "r"),
        ("personality flag", "a QNX client", "r"), ("receive ID", "qnxd's thread", "y")]
x = 40
for name, val, cls in cols:
    f.box(cls, x, 220, 118, 48, 2); f.text(x + 59, 240, name, "s c"); f.text(x + 59, 258, val, "ms c")
    x += 120
f.text(400, 292, "Everything that identifies the client is the client's. Only the receive ID, which names where the reply goes, is qnxd's.", "s c")
f.text(400, 308, "An event the server delivers later through that receive ID still goes to the client.", "s c")
f.save("ch33-identity.svg")

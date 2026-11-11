from svg import Fig
f = Fig(800, 440)
L, P, S = 130, 400, 670
f.box("k", L - 110, 20, 220, 34); f.text(L, 42, "libsys, in open()", "h c")
f.box("k", P - 110, 20, 220, 34); f.text(P, 42, "pathname manager", "h c")
f.box("u", S - 110, 20, 220, 34); f.text(S, 42, "tmpfs", "h c")
for x in (L, P, S): f.line([(x, 54), (x, 330)], "life")
def msg(y, a, b, n, t1, t2="", cls="lnk", m="ak"):
    f.arrow([(a, y), (b, y)], cls, m)
    mid = (a + b) / 2
    f.num(min(a, b) + 18, y - 14, n, "#2c5d9b" if cls == "lnk" else "#3e7b34")
    f.text(min(a, b) + 34, y - 10, t1, "s")
    if t2: f.text(min(a, b) + 34, y + 14, t2, "ms")
msg(92, L, P, 1, "connect request, on the connection to process 1", '"/tmp/notes", open, read-only')
msg(150, P, L, 2, "a link: the servers whose prefix matches", "{ tmpfs's process and channel, the prefix's length }", "lng", "ag")
f.box("wk", L - 100, 176, 200, 40, 3); f.num(L - 84, 196, 3, "#2c5d9b"); f.text(L + 10, 193, "ConnectAttach(process,", "ms c"); f.text(L + 10, 207, "channel): connection 3", "ms c")
msg(252, L + 100, S, 4, "the same request on the new connection, the prefix removed", '"notes", open, read-only')
msg(310, S, L, 5, "status 0: the file is open", "", "lng", "ag")
f.box("g", L - 100, 322, 200, 26, 3); f.text(L, 340, "open() returns 3", "ms c")
# the other answers
f.box("w", 20, 366, 760, 62, 3); f.text(32, 384, "Other answers a server may give in step 5", "b")
f.text(32, 402, "ENOENT: libsys detaches and tries the next server of the list; with none left, open() fails with it.", "s")
f.text(32, 418, "A link with a path and no servers: the name is a symbolic link; start again at step 1 with its target, at most 16 times.", "s")
f.save("ch24-connect.svg")

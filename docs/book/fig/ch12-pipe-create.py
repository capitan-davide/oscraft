from svg import Fig
f = Fig(800, 470)
px, kx, sx = 120, 400, 680
f.box("u", px - 100, 14, 200, 40); f.text(px, 31, "a program in pipe()", "c"); f.text(px, 46, "libsys", "s c")
f.box("k", kx - 80, 14, 160, 40); f.text(kx, 39, "kernel", "c")
f.box("k", sx - 100, 14, 200, 40); f.text(sx, 31, "the pipe server", "c"); f.text(sx, 46, "user/pipe", "ms c")
for x in (px, kx, sx): f.line([(x, 54), (x, 440)], "life")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1 + (2 if x1 < x0 else -2), y)], cls, m)
    if x0 < x1:
        f.num(x0 + 18, y - 13, n, col); f.text(x0 + 34, y - 8, text, "s")
    else:
        f.num(x0 - 18, y - 13, n, col); f.text(x0 - 34, y - 8, text, "s e")
step(1, 92, px, sx, "open \"/\" as a pipe, on a new connection: descriptor 3")
step(2, 134, sx, kx, "PipeOpen(client, new pipe, sizes)")
f.box("y", kx - 86, 142, 172, 44, 3); f.text(kx, 160, "makes the pipe;", "s c"); f.text(kx, 174, "binds 3 to it as the read end", "s c")
f.arrow([(kx + 88, 176), (sx - 2, 176)], "lng", "ag"); f.text(sx - 8, 190, "the pipe's ID", "s e green")
f.arrow([(sx, 214), (px + 2, 214)], "lng", "ag"); f.text(px + 34, 208, "reply: opened", "s green")
step(3, 256, px, kx, "a second connection to the server: descriptor 4")
step(4, 298, px, sx, "on 4: \"open the other end of what 3 is\"")
step(5, 340, sx, kx, "PipeOpen(client, that pipe, other side)")
f.box("y", kx - 86, 348, 172, 30, 3); f.text(kx, 367, "binds 4 to it as the write end", "s c")
f.arrow([(sx, 400), (px + 2, 400)], "lng", "ag"); f.text(px + 34, 394, "reply: opened", "s green")
f.text(px + 8, 426, "pipe() returns {3, 4}", "s")
f.text(400, 458, "The server is asked twice and then steps aside: it never sees a byte of what flows through the pipe.", "s c")
f.save("ch12-pipe-create.svg")

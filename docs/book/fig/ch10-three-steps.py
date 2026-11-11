from svg import Fig
f = Fig(800, 400)
def panel(x0, title, busy):
    cx, sx = x0 + 90, x0 + 290
    f.text(x0 + 190, 26, title, "h c")
    f.box("u", cx - 60, 40, 120, 30); f.text(cx, 60, "client", "c")
    f.box("u", sx - 60, 40, 120, 30); f.text(sx, 60, "server", "c")
    f.line([(cx, 70), (cx, 360)], "life"); f.line([(sx, 70), (sx, 360)], "life")
    if not busy:
        # server waits first
        mid = (cx + sx) // 2
        f.num(sx + 62, 104, 1, "#333333"); f.text(sx + 76, 100, "receive:", "s"); f.text(sx + 76, 113, "nothing yet", "s")
        f.box("y", sx - 45, 84, 90, 56, 3); f.text(sx, 108, "RECEIVE-", "s c"); f.text(sx, 121, "blocked", "s c")
        y_send = 150
        f.arrow([(cx, y_send), (sx - 2, y_send)], "lnk", "ak"); f.num(mid - 22, y_send - 14, 2, "#2c5d9b"); f.text(mid - 8, y_send - 10, "send", "s")
        f.box("y", cx - 45, y_send + 6, 90, 150, 3); f.text(cx, y_send + 76, "REPLY-", "s c"); f.text(cx, y_send + 89, "blocked", "s c")
        f.box("g", sx - 45, y_send + 6, 90, 130, 3); f.text(sx, y_send + 66, "works on", "s c"); f.text(sx, y_send + 79, "the request", "s c")
        y_rep = y_send + 150
        f.arrow([(sx, y_rep), (cx + 47, y_rep)], "lng", "ag"); f.num(mid - 22, y_rep - 14, 3, "#3e7b34"); f.text(mid - 8, y_rep - 10, "reply", "s")
        f.box("g", cx - 45, y_rep + 8, 90, 40, 3); f.text(cx, y_rep + 32, "runs again", "s c")
        f.box("y", sx - 45, y_rep + 8, 90, 40, 3); f.text(sx, y_rep + 25, "receive:", "s c"); f.text(sx, y_rep + 38, "waits again", "s c")
    else:
        f.box("g", sx - 45, 84, 90, 110, 3); f.text(sx, 132, "busy with", "s c"); f.text(sx, 145, "another", "s c"); f.text(sx, 158, "request", "s c")
        y_send = 110
        mid = (cx + sx) // 2
        f.arrow([(cx, y_send), (sx - 47, y_send)], "lnk", "ak"); f.num(mid - 22, y_send - 14, 1, "#2c5d9b"); f.text(mid - 8, y_send - 10, "send", "s")
        f.box("r", cx - 45, y_send + 6, 90, 84, 3); f.text(cx, y_send + 44, "SEND-", "s c"); f.text(cx, y_send + 57, "blocked", "s c")
        y_rcv = 204
        f.text(sx + 52, y_rcv + 4, "", "s")
        f.arrow([(sx - 2, y_rcv), (cx + 47, y_rcv)], "ln", "a", True); f.num(mid - 62, y_rcv - 14, 2, "#333333"); f.text(mid - 48, y_rcv - 10, "receive: takes it", "s")
        f.box("y", cx - 45, y_rcv + 6, 90, 96, 3); f.text(cx, y_rcv + 50, "REPLY-", "s c"); f.text(cx, y_rcv + 63, "blocked", "s c")
        f.box("g", sx - 45, y_rcv + 6, 90, 76, 3); f.text(sx, y_rcv + 40, "works on", "s c"); f.text(sx, y_rcv + 53, "the request", "s c")
        y_rep = 300
        f.arrow([(sx - 47, y_rep), (cx + 47, y_rep)], "lng", "ag"); f.num(mid - 22, y_rep - 14, 3, "#3e7b34"); f.text(mid - 8, y_rep - 10, "reply", "s")
        f.box("g", cx - 45, y_rep + 8, 90, 40, 3); f.text(cx, y_rep + 32, "runs again", "s c")
        f.box("y", sx - 45, y_rep + 8, 90, 40, 3); f.text(sx, y_rep + 25, "receive:", "s c"); f.text(sx, y_rep + 38, "waits again", "s c")
panel(10, "The server is waiting", False)
f.line([(400, 20), (400, 360)], "life")
panel(410, "The server is busy", True)
f.text(400, 386, "Time runs downwards. The client is blocked from its send to the reply; the server is blocked only while it has nothing to do.", "s c")
f.save("ch10-three-steps.svg")

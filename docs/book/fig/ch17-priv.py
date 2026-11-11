from svg import Fig
f = Fig(800, 430)
C, K = 70, 710
f.box("u", 10, 14, 130, 30); f.text(C, 34, "a program, user 1001", "s c")
f.box("g", 180, 14, 400, 30); f.text(380, 34, "netd", "c")
f.box("k", 640, 14, 140, 30); f.text(K, 34, "kernel", "c")
f.line([(C, 44), (C, 410)], "life"); f.line([(K, 44), (K, 410)], "life")
f.box("wk", 180, 56, 190, 354, 3); f.text(275, 74, "bsdkern's privilege.rs", "b c")
f.box("g", 390, 56, 190, 354, 3); f.text(485, 74, "FreeBSD's network code", "b c")
def note(x, y, lines, cls="w", w=170, tc="s c"):
    h = 10 + 14 * len(lines)
    f.box(cls, x - w / 2, y, w, h, 3); f.lines(x, y + 15, lines, tc, 14)
L, R = 275, 485
note(L, 86, ["at start: creates the ability", "network/privport by name"])
f.arrow([(360, 105), (K - 2, 105)], "lnk", "ak", True); f.text(610, 99, "process 1 gives it an ID", "s c")
# 1
f.arrow([(C, 152), (178, 152)]); f.num(C + 14, 140, 1); f.text(C + 28, 144, "bind(), port 80", "s")
note(L, 162, ["this thread now works", "for that client"])
# 2
f.arrow([(360, 181), (398, 181)]); f.num(380, 168, 2)
note(R, 162, ["in_pcbbind(): a port below", "1024 needs a privilege"])
note(R, 210, ["priv_check(td, PRIV_", "NETINET_RESERVEDPORT)"], "w", 170, "ms c")
# 3
f.arrow([(400, 229), (362, 229)], "lng", "ag"); f.num(380, 216, 3, "#3e7b34")
note(L, 210, ["its table: that privilege", "is network/privport"])
# 4
f.arrow([(L, 248), (L, 272), (K - 2, 272)], "lnk", "ak"); f.num(600, 260, 4, "#2c5d9b"); f.text(590, 266, "ConnectClientInfoAble(client, ability)", "s e")
note(K, 282, ["the client's abilities:", "not root, not", "granted: no"], "wk", 136)
# 5
f.arrow([(K - 68, 320), (L + 40, 320), (L + 40, 338)], "lnr", "ar", True); f.num(600, 308, 5, "#aa2222"); f.text(590, 314, "not held", "s e")
note(L, 340, ["priv_check() returns EPERM"])
f.arrow([(360, 352), (398, 352)], "lnr", "ar")
note(R, 340, ["bind fails: EACCES"])
f.arrow([(400, 384), (C + 2, 384)], "lnr", "ar"); f.text(125, 378, "EACCES", "s c red")
f.save("ch17-priv.svg")

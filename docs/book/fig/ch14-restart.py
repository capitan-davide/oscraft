from svg import Fig
f = Fig(800, 450)
def panel(x0, title, restart):
    P, K = x0 + 90, x0 + 280
    f.text(x0 + 185, 24, title, "h c")
    f.box("u", P - 70, 36, 140, 28); f.text(P, 55, "thread", "c")
    f.box("k", K - 70, 36, 140, 28); f.text(K, 55, "kernel", "c")
    f.line([(P, 64), (P, 392)], "life"); f.line([(K, 64), (K, 392)], "life")
    f.arrow([(P, 90), (K - 2, 90)]); f.text((P + K) / 2, 84, "read(): MsgSendv()", "s c")
    f.box("y", K - 60, 98, 120, 50, 3); f.text(K, 119, "blocked, waiting", "s c"); f.text(K, 133, "for the server", "s c")
    f.arrow([(K + 100, 160), (K + 2, 160)], "lnr", "ar"); f.text(K + 100, 152, "signal", "s e red")
    f.box("wk", K - 80, 168, 160, 36); f.text(K, 183, "the wait ends: EINTR", "s c"); f.text(K, 197, "pc moved back to the call" if restart else "result = EINTR", "s c")
    if restart:
        f.text(K, 197, "", "s")
    f.arrow([(K, 222), (P + 2, 222)], "lnr", "ar"); f.text((P + K) / 2, 216, "enter the handler", "s c red")
    f.box("u", P - 55, 230, 110, 40, 3); f.text(P, 254, "handler runs", "s c")
    f.arrow([(P, 286), (K - 2, 286)]); f.text((P + K) / 2, 280, "SignalReturn()", "s c")
    if restart:
        f.arrow([(K, 320), (P + 2, 320)], "lng", "ag"); f.text((P + K) / 2, 314, "resume at the call", "s c green")
        f.arrow([(P, 356), (K - 2, 356)], "lng", "ag"); f.text((P + K) / 2, 350, "the same MsgSendv()", "s c green")
        f.text(P, 384, "read() does not see EINTR", "s c")
    else:
        f.arrow([(K, 320), (P + 2, 320)], "lnr", "ar"); f.text((P + K) / 2, 314, "read() returns -1, EINTR", "s c red")
        f.text(P, 350, "the program must", "s c"); f.text(P, 364, "check and retry", "s c")
panel(10, "A handler without SA_RESTART", False)
f.line([(400, 14), (400, 396)], "life")
panel(410, "A handler with SA_RESTART", True)
f.text(400, 420, "With SA_RESTART the kernel does not complete the interrupted call. It moves the program counter back by one", "s c")
f.text(400, 436, "instruction before saving the context, so that returning from the handler makes the same kernel call again.", "s c")
f.save("ch14-restart.svg")

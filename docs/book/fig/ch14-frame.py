from svg import Fig
f = Fig(800, 470)
f.text(20, 24, "The user stack while a handler runs", "h")
X, W = 20, 330
f.box("g", X, 40, W, 50, 0); f.text(X + W / 2, 69, "the interrupted code's frames", "s c")
f.box("y", X, 90, W, 170, 0)
f.text(X + W / 2, 110, "ucontext_t (944 bytes)", "b c")
rows = [("+0", "uc_sigmask: the mask to restore (16 bytes)"), ("+16", "x0 to x30, sp, pc, pstate"), ("+288", "v0 to v31, FPSR, FPCR, flags")]
for i, (o, t) in enumerate(rows):
    y = 122 + i * 42
    f.box("w", X + 14, y, W - 28, 34, 2); f.text(X + 24, y + 21, o, "ms"); f.text(X + 70, y + 21, t, "s")
f.box("u", X, 260, W, 110, 0)
f.text(X + W / 2, 280, "frame (96 bytes)", "b c")
rows = [("+0", "siginfo_t (80 bytes)"), ("+80", "the handler's address"), ("+88", "the address of the ucontext_t")]
for i, (o, t) in enumerate(rows):
    y = 288 + i * 26
    f.box("w", X + 14, y, W - 28, 22, 2); f.text(X + 24, y + 15, o, "ms"); f.text(X + 70, y + 15, t, "s")
f.box("w", X, 370, W, 44, 0); f.text(X + W / 2, 396, "the trampoline's and the handler's frames", "s c")
f.text(X + W + 8, 94, "sp before", "s"); f.text(X + W + 8, 374, "sp = x0", "s"); f.text(X + W + 8, 388, "at entry", "s")
f.arrow([(X + W - 14, 353), (X + W + 40, 353), (X + W + 40, 200), (X + W + 2, 200)], "ln", "a", True)
# steps
SX, SW = 480, 300
S = [("k", "kernel: deliver()", "takes the lowest pending, unblocked signal"),
     ("k", "kernel: enter_handler()", "writes both structures; pc = trampoline"),
     ("g", "libsys: __oscraft_sigstub(frame)", "calls handler(signo, &siginfo, &ucontext)"),
     ("u", "the program's handler", "runs with the signal blocked; returns"),
     ("g", "libsys: SignalReturn(frame)", "a kernel call that does not return"),
     ("k", "kernel: signal_return()", "registers, FP state and mask from the ucontext")]
for i, (c, a, b) in enumerate(S):
    y = 40 + i * 64
    f.box(c, SX, y, SW, 46); f.num(SX - 14, y + 23, i + 1)
    f.text(SX + SW / 2, y + 19, a, "c"); f.text(SX + SW / 2, y + 35, b, "s c")
    if i < 5:
        f.arrow([(SX + SW / 2, y + 46), (SX + SW / 2, y + 62)])
f.text(400, 446, "The context is in the program's memory, and the handler may change it: what SignalReturn() restores is what it finds,", "s c")
f.text(400, 462, "except the processor mode, which stays user mode whatever the saved pstate says.", "s c")
f.save("ch14-frame.svg")

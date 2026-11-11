from svg import Fig
f = Fig(800, 430)
X, W = 30, 430
rows = [
    ("u", "the server's own functions", "open, read, write, devctl ... only those it needs", "what the device or file system does"),
    ("k", "iofunc", "user/resmgr/src/iofunc.rs", "attributes and open-control blocks; a default for every request"),
    ("k", "resmgr", "user/resmgr/src/resmgr.rs", "paths attached, connections bound to open files, the replies"),
    ("k", "dispatch", "user/resmgr/src/dispatch.rs", "the channel, the handlers by message type, the receive loop"),
    ("k", "kercall", "user/kercall/", "kernel calls as functions; the client side of the protocol"),
]
y = 20
for cls, name, sub, note in rows:
    f.box(cls, X, y, W, 50); f.text(X + 16, y + 22, name, "b"); f.text(X + 16, y + 39, sub, "ms")
    f.lines(X + W + 20, y + 22, [note[:46].rsplit(" ", 1)[0] if len(note) > 46 else note] + ([note[len(note[:46].rsplit(" ", 1)[0]) + 1:]] if len(note) > 46 else []), "s", 15)
    y += 58
# FreeBSD's libc beside the lower layers
f.box("g", X, y, W, 44); f.text(X + 16, y + 20, "FreeBSD's libc and libthr, Rust's std", "b"); f.text(X + 16, y + 36, "memory, threads, mutexes, errno; over libsys", "s")
f.lines(X + W + 20, y + 20, ["a server is an ordinary FreeBSD program,", "linked statically"], "s", 15)
y += 62
f.line([(10, y - 9), (790, y - 9)], "life")
f.box("k", X, y, W, 40); f.text(X + W / 2, y + 24, "the kernel: channels, messages, pulses", "b c")
f.text(X + W + 20, y + 24, "entered by a supervisor call", "s")
f.save("ch25-layers.svg")

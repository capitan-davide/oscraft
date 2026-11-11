from svg import Fig
f = Fig(800, 372)
f.text(140, 24, "What a compiled program assumes", "h c")
f.text(540, 24, "A QNX program and a FreeBSD program disagree, for example, on", "h c")
rows = [
    ("How it is started", "the stack and registers at its first instruction",
     "who prepares the thread-local block; what the auxiliary vector holds"),
    ("How it enters the kernel", "the trap, the call number, the error",
     "nothing here: both use the same kernel calls (svc 0x51, number in w8)"),
    ("Numbers", "error numbers, signal numbers, flag bits",
     "EAGAIN: 11 or 35.  SIGUSR1: 16 or 30.  O_NONBLOCK: 0x80 or 0x4"),
    ("Layouts", "the fields and size of each structure",
     "struct stat: 120 or 224 bytes.  struct termios: 76 or 44 bytes"),
    ("Behaviour", "which error, in what order, how soon",
     "which of two pending signals is delivered first"),
    ("Names", "paths it opens without being told to",
     "/bin/sh, its dynamic loader, its C library, /proc/<pid>/as"),
]
y = 40
for i, (name, what, ex) in enumerate(rows):
    cls = "g" if i == 1 else "y"
    f.box("k", 20, y, 240, 46); f.text(140, y + 20, name, "b c"); f.text(140, y + 36, what, "s c")
    f.box(cls, 280, y, 500, 46); f.text(530, y + 27, ex, "s c")
    f.arrow([(260, y + 23), (278, y + 23)])
    y += 52
f.text(400, 364, "Green: already the same. Yellow: must be converted or provided for a QNX program.", "s c")
f.save("ch32-abi-layers.svg")

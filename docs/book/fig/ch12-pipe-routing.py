from svg import Fig
f = Fig(800, 400)
# process
f.box("u", 20, 40, 150, 150); f.text(95, 60, "a program", "c")
f.box("wu", 36, 76, 118, 30, 2); f.text(95, 95, "fd 3: read()", "s c")
f.box("wu", 36, 126, 118, 30, 2); f.text(95, 145, "fd 4: write()", "s c")
f.text(95, 178, "its connections", "s c")
# kernel frame
f.box("frame", 200, 14, 370, 372); f.text(385, 32, "kernel", "h c")
f.box("k", 216, 66, 150, 50, 3); f.text(291, 85, "connection 3", "s c b"); f.text(291, 101, "bound: read end", "s c")
f.box("k", 216, 122, 150, 50, 3); f.text(291, 141, "connection 4", "s c b"); f.text(291, 157, "bound: write end", "s c")
f.arrow([(154, 91), (214, 91)], "ln", "a"); f.arrow([(154, 141), (214, 147)], "ln", "a")
f.box("y", 410, 56, 146, 130, 3); f.text(483, 74, "the pipe", "c")
f.lines(420, 94, ["buffer: 65536 bytes", "atomic size: 512", "readers: 1  writers: 1", "threads waiting to read", "threads waiting to write", "armed notifications"], "s", 15)
f.arrow([(366, 91), (408, 100)], "lnk", "ak"); f.arrow([(366, 147), (408, 138)], "lnk", "ak")
# decision
f.box("k", 216, 226, 340, 50, 3); f.text(386, 246, "MsgSendv() on a connection: is it bound to a pipe,", "s c"); f.text(386, 262, "and is the message a read or a write?", "s c")
f.arrow([(300, 276), (300, 316)], "lng", "ag"); f.text(308, 300, "yes", "s green")
f.box("g", 216, 318, 170, 52, 3); f.lines(301, 336, ["the kernel copies to or from", "the pipe's buffer, in the", "caller's own thread"], "s c", 13)
f.arrow([(556, 251), (628, 251)], "ln", "a"); f.text(592, 243, "no", "s c")
# server
f.box("k", 630, 40, 150, 330); f.text(705, 60, "the pipe server", "c"); f.text(705, 76, "user/pipe", "ms c")
f.text(705, 104, "receives only:", "s c b")
f.lines(642, 126, ["the two opens", "a copied descriptor", "  (dup, fork)", "poll() and select()", "  asking and arming", "fcntl(): O_NONBLOCK", "fstat(), FIONREAD", "fpathconf()", "lseek(): refused", "close()"], "s", 16)
f.lines(642, 300, ["and answers with", "PipeOpen(), PipeCtl()", "and PipeClose()"], "s", 15)
f.save("ch12-pipe-routing.svg")

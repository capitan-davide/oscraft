from svg import Fig
f = Fig(800, 470)
# three lanes
f.box("u", 20, 20, 250, 430); f.text(145, 42, "a FreeBSD program", "h c")
f.box("k", 290, 20, 220, 430); f.text(400, 42, "the kernel", "h c")
f.box("u", 530, 20, 250, 430); f.text(655, 42, "a server: tmpfs", "h c")
# program internals
f.box("wu", 40, 60, 210, 44, 3); f.text(145, 79, "n = read(fd, buf, 4096);", "m c"); f.text(145, 95, "the application", "s c")
f.box("wu", 40, 124, 210, 44, 3); f.text(145, 143, "FreeBSD's libc", "c"); f.text(145, 159, "unchanged: calls __sys_read()", "s c")
f.box("g", 40, 188, 210, 96, 3); f.text(145, 207, "libsys", "b c"); f.text(145, 224, "builds a 16-byte request:", "s c")
f.text(145, 242, "{ _IO_READ, nbytes = 4096 }", "ms c"); f.text(145, 260, "reply goes straight into buf", "s c"); f.text(145, 276, "MsgSendv(fd, request, buf)", "ms c")
f.arrow([(145, 104), (145, 122)]); f.arrow([(145, 168), (145, 186)])
# kernel
f.box("wk", 310, 196, 180, 62, 3); f.text(400, 215, "the connection fd leads", "s c"); f.text(400, 231, "to tmpfs's channel", "s c"); f.text(400, 248, "copy the request", "c")
f.box("y", 310, 276, 180, 40, 3); f.text(400, 293, "the program's thread:", "s c"); f.text(400, 308, "REPLY-blocked", "s c")
f.box("wk", 310, 352, 180, 62, 3); f.text(400, 371, "copy the bytes into buf,", "c"); f.text(400, 388, "the status is read()'s result", "s c"); f.text(400, 404, "wake the program", "s c")
# server
f.box("wu", 550, 196, 210, 44, 3); f.text(655, 215, "MsgReceive(chid, ...)", "ms c"); f.text(655, 231, "a server thread wakes", "s c")
f.box("wu", 550, 262, 210, 60, 3); f.text(655, 287, "the read handler", "c"); f.text(655, 304, "finds the file's bytes", "s c")
f.box("wu", 550, 352, 210, 44, 3); f.text(655, 371, "MsgReply(rcvid, n, bytes)", "ms c"); f.text(655, 387, "and back to MsgReceive()", "s c")
f.arrow([(655, 240), (655, 260)]); f.arrow([(655, 322), (655, 350)])
# cross arrows
f.arrow([(250, 228), (308, 228)], "lnk", "ak"); f.num(279, 216, 1, "#2c5d9b")
f.arrow([(490, 218), (548, 218)], "lnk", "ak"); f.num(519, 206, 2, "#2c5d9b")
f.arrow([(548, 374), (492, 374)], "lng", "ag"); f.num(519, 362, 3, "#3e7b34")
f.arrow([(308, 384), (252, 384)], "lng", "ag"); f.num(279, 372, 4, "#3e7b34")
f.box("wu", 40, 362, 210, 44, 3); f.text(145, 381, "read() returns n", "m c"); f.text(145, 397, "buf holds the file's bytes", "s c")
f.text(400, 440, "one kernel call from the program, two copies, no kernel buffer", "s c")
f.save("ch10-read-as-message.svg")

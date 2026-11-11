from svg import Fig
f = Fig(800, 360)
f.box("frame", 20, 40, 500, 200); f.text(34, 58, "session 28681: everything started from the console's shell", "b")
f.box("u", 40, 76, 120, 50); f.text(100, 96, "sh  28681", "c"); f.text(100, 112, "session leader", "s c")
f.box("frame", 190, 70, 310, 70); f.text(200, 86, "process group 32778: sleep 100 &", "s")
f.box("u", 204, 94, 110, 36); f.text(259, 116, "sleep  32778", "s c")
f.text(410, 116, "background", "s c")
f.box("g", 190, 152, 310, 76, 3); f.text(200, 168, "process group 36875: ls | wc", "s"); f.text(490, 168, "foreground", "s e green")
f.box("u", 204, 178, 110, 36); f.text(259, 200, "ls  36875", "s c")
f.box("u", 330, 178, 110, 36); f.text(385, 200, "wc  36876", "s c")
# terminal
f.box("k", 580, 40, 200, 110); f.text(680, 60, "console /dev/text", "b c"); f.text(680, 76, "(process 1's devtext)", "s c")
f.box("wk", 594, 88, 172, 50, 3); f.lines(680, 106, ["foreground group:", "36875"], "s c", 15)
f.box("n", 580, 186, 200, 40); f.text(680, 204, "keyboard: Ctrl-C", "c"); f.text(680, 219, "(the serial port)", "s c")
f.arrow([(680, 186), (680, 152)]); 
f.arrow([(594, 124), (540, 124), (540, 190), (502, 190)], "lnr", "ar"); f.text(548, 176, "SIGINT", "s red")
f.text(548, 190, "to each", "s red")
# process 1 record
f.box("k", 20, 262, 760, 84); f.text(34, 282, "what process 1 keeps", "b")
f.box("wk", 34, 292, 350, 44, 3); f.lines(44, 308, ["in each process: its group and its session", "(the kernel's Process structure)"], "s", 14)
f.box("wk", 400, 292, 366, 44, 3); f.lines(410, 308, ["per session: its controlling terminal, as a connection", "of process 1's own, closed when the leader ends"], "s", 14)
f.save("ch18-session.svg")

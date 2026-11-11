from svg import Fig
f = Fig(800, 300)
st = [("k", 20, "created", ["an ID, an empty", "address space,", "inherited handles"]),
      ("g", 215, "running", ["memory, threads,", "handles, timers:", "everything"]),
      ("r", 455, "zombie", ["an ID and an", "exit status;", "nothing else"]),
      ("n", 680, "gone", ["the ID may", "be given to", "another process"])]
for c, x, name, what in st:
    w = 150 if name != "gone" else 110
    f.box(c, x, 60, w, 96); f.text(x + w / 2, 82, name, "h c")
    f.lines(x + w / 2, 104, what, "s c", 15)
f.arrow([(170, 108), (213, 108)]); f.text(192, 98, "first", "s c"); f.text(192, 126, "thread", "s c")
f.arrow([(365, 96), (453, 96)]); f.text(409, 74, "exit(), or a", "s c"); f.text(409, 88, "fatal signal", "s c")
f.text(409, 116, "last thread", "s c"); f.text(409, 130, "ends", "s c")
f.arrow([(605, 108), (678, 108)]); f.text(642, 98, "parent's", "s c"); f.text(642, 126, "wait()", "s c")
# stopped
f.box("y", 215, 214, 150, 50); f.text(290, 236, "stopped", "h c"); f.text(290, 252, "no thread runs", "s c")
f.arrow([(260, 156), (260, 212)]); f.text(254, 190, "SIGSTOP", "s e")
f.arrow([(320, 214), (320, 158)]); f.text(326, 190, "SIGCONT", "s")
# what is released
f.line([(455, 180), (605, 180)], "lnr"); f.line([(455, 174), (455, 186)], "lnr"); f.line([(605, 174), (605, 186)], "lnr")
f.lines(530, 200, ["memory freed, handles closed,", "parent notified (SIGCHLD)"], "s c", 14)
f.save("ch17-life.svg")

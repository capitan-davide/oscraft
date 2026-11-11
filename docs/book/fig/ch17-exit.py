from svg import Fig
f = Fig(800, 490)
cols = [("parent", "u", 80), ("the ending process", "r", 290), ("process 1", "k", 500), ("servers and clients", "u", 700)]
for name, c, x in cols:
    f.box(c, x - 75, 14, 150, 30); f.text(x, 34, name, "c")
    f.line([(x, 44), (x, 472)], "life")
P, E, K, S = 80, 290, 500, 700
def note(x, y, lines, cls="w", w=190):
    h = 10 + 14 * len(lines)
    f.box(cls, x - w / 2, y, w, h, 3); f.lines(x, y + 15, lines, "s c", 14)
# 0 exit
note(E, 54, ["_exit() or a fatal signal:", "every thread is told to end"], "y")
note(E, 100, ["the last thread ends:", "the process is a zombie"], "r")

note(P, 100, ["may be blocked", "in wait()"], "y", 130)
f.arrow([(E, 168), (P + 2, 168)], "ln", "a"); f.num(E - 16, 156, 1); f.text(E - 30, 160, "SIGCHLD; its CPU time", "s e")
f.num(E - 100, 203, 2); note(E + 8, 184, ["timers and interrupt", "attachments removed"], "w", 170)
f.arrow([(E, 250), (K - 2, 250)], "lnk", "ak"); f.num(E + 20, 238, 3, "#2c5d9b"); f.text(E + 34, 242, "pulse: process ended", "s")
f.arrow([(E, 284), (S - 2, 284)], "ln", "a"); f.num(E + 20, 272, 4); f.text(E + 34, 276, "connections close: each server gets a disconnect pulse", "s")
f.arrow([(E, 318), (S - 2, 318)], "lnr", "ar"); f.num(E + 20, 306, 5, "#aa2222"); f.text(E + 34, 310, "channels destroyed: blocked clients fail with ESRCH", "s")
f.line([(E - 30, 334), (E + 30, 334)], "lnr"); f.text(E, 350, "no thread left", "s c red")
note(K, 340, ["frees its memory and page tables,", "open files, registered names,", "its session's terminal"], "wk", 200)
note(K, 404, ["answers a held wait, removes", "the zombie (its slot is free),", "adopts its children"], "wk", 200)
f.arrow([(K - 100, 430), (P + 2, 430)], "lng", "ag"); f.text(240, 422, "wait() returns the exit status", "s c")
f.save("ch17-exit.svg")

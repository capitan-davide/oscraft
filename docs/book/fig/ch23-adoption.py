from svg import Fig
f = Fig(800, 380)
def col(x, title, sub):
    f.text(x + 120, 24, title, "h c"); f.text(x + 120, 40, sub, "s c")
    f.box("u", x + 20, 52, 200, 34); f.text(x + 120, 73, "the foreign program", "c")
X = [20, 280, 540]
# A: in the kernel
x = X[0]; col(x, "A. In the kernel", "a second system-call table")
f.box("u", x + 20, 96, 200, 50); f.text(x + 120, 116, "its own C library,", "c"); f.text(x + 120, 132, "whole and unchanged", "s c")
f.arrow([(x + 120, 86), (x + 120, 95)])
f.arrow([(x + 120, 146), (x + 120, 196)]); f.text(x + 128, 168, "trap, with the foreign", "s"); f.text(x + 128, 182, "system's call numbers", "s")
f.box("k", x + 20, 198, 200, 160); f.text(x + 120, 216, "the kernel", "b c")
f.box("y", x + 34, 226, 172, 56, 3); f.text(x + 120, 246, "emulation: foreign numbers", "s c"); f.text(x + 120, 260, "and layouts, converted", "s c"); f.text(x + 120, 274, "call by call", "s c")
f.box("wk", x + 34, 300, 172, 44, 3); f.text(x + 120, 319, "the kernel's own", "s c"); f.text(x + 120, 333, "implementation", "s c")
f.arrow([(x + 120, 282), (x + 120, 298)])
# B: in the library
x = X[1]; col(x, "B. In the library", "a new bottom layer")
f.box("u", x + 20, 96, 200, 100); f.text(x + 120, 116, "its own C library", "c")
f.arrow([(x + 120, 86), (x + 120, 95)])
f.box("y", x + 34, 130, 172, 56, 3); f.text(x + 120, 150, "system-call layer, rewritten:", "s c"); f.text(x + 120, 164, "the same functions, made of", "s c"); f.text(x + 120, 178, "the host kernel's calls", "s c")
f.arrow([(x + 120, 196), (x + 120, 246)]); f.text(x + 128, 218, "the host kernel's", "s"); f.text(x + 128, 232, "own calls", "s")
f.box("k", x + 20, 248, 200, 110); f.text(x + 120, 268, "the kernel", "b c")
f.text(x + 120, 290, "no second table;", "s c"); f.text(x + 120, 306, "but what it writes into the", "s c"); f.text(x + 120, 322, "program's memory must have", "s c"); f.text(x + 120, 338, "the foreign layouts", "s c")
# C: in a process
x = X[2]; col(x, "C. In a process", "a translator between programs")
f.box("u", x + 20, 96, 200, 50); f.text(x + 120, 116, "its own C library,", "c"); f.text(x + 120, 132, "whole and unchanged", "s c")
f.arrow([(x + 120, 86), (x + 120, 95)])
f.arrow([(x + 120, 146), (x + 120, 176)])
f.box("k", x + 20, 178, 200, 40); f.text(x + 120, 196, "the kernel", "b c"); f.text(x + 120, 211, "only carries the requests", "s c")
f.arrow([(x + 120, 218), (x + 120, 240)])
f.box("y", x + 20, 242, 200, 50, 3); f.text(x + 120, 262, "a translation process:", "s c"); f.text(x + 120, 278, "converts each request and reply", "s c")
f.arrow([(x + 120, 292), (x + 120, 312)])
f.box("wk", x + 20, 314, 200, 44, 3); f.text(x + 120, 333, "the system's own services,", "s c"); f.text(x + 120, 347, "in their own layouts", "s c")
f.save("ch23-adoption.svg")

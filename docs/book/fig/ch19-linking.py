from svg import Fig
f = Fig(800, 400)
# static
f.text(160, 24, "Statically linked", "h c")
f.box("u", 30, 40, 260, 110); f.text(160, 58, "the program's file", "b c")
f.box("wu", 45, 68, 230, 30, 2); f.text(160, 87, "the program's own code", "c")
f.box("g", 45, 104, 230, 34, 2); f.text(160, 119, "the library functions it uses,", "s c"); f.text(160, 132, "copied in by the linker", "s c")
f.arrow([(160, 150), (160, 206)]); f.num(180, 178, 1)
f.box("k", 30, 208, 260, 50); f.text(160, 230, "one image in memory", "c"); f.text(160, 246, "nothing left to resolve", "s c")
f.text(30, 300, "1  The kernel maps the segments and starts", "s"); f.text(30, 314, "    the program at its entry point.", "s")
f.line([(320, 16), (320, 384)], "life")
# dynamic
f.text(565, 24, "Dynamically linked", "h c")
cols = [(350, 150), (512, 126), (650, 130)]
f.box("u", 350, 40, 150, 84); f.text(425, 58, "the program's file", "b c")
f.box("wu", 360, 66, 130, 22, 2); f.text(425, 82, "its own code", "s c")
f.box("y", 360, 92, 130, 24, 2); f.text(425, 108, "names its interpreter", "s c")
f.box("g", 512, 60, 126, 44); f.text(575, 79, "the dynamic", "c"); f.text(575, 94, "loader's file", "c")
f.box("g", 650, 60, 130, 44); f.text(715, 79, "shared", "c"); f.text(715, 94, "libraries' files", "c")
f.arrow([(490, 104), (510, 90)], "ln", "a", True)
f.box("k", 340, 190, 450, 78)
f.text(565, 206, "the process's memory", "b c")
f.box("wu", 350, 216, 150, 40, 2); f.text(425, 240, "program", "c")
f.box("g", 512, 216, 126, 40, 2); f.text(575, 240, "dynamic loader", "c")
f.box("g", 650, 216, 130, 40, 2); f.text(715, 240, "libraries", "c")
f.arrow([(425, 124), (425, 214)]); f.num(441, 158, 1)
f.arrow([(575, 104), (575, 214)]); f.num(591, 158, 1)
f.arrow([(715, 104), (715, 214)], "lng", "ag"); f.num(731, 158, 3, "#3e7b34")
f.arrow([(575, 258), (575, 280), (425, 280), (425, 258)], "lng", "ag"); f.num(500, 280, 4, "#3e7b34")
L = ["1  The kernel maps the program and the interpreter it names.",
     "2  The kernel starts the loader, not the program.",
     "3  The loader maps each library the program needs and fills in",
     "    the addresses the program left open.",
     "4  The loader jumps to the program's entry point."]
f.lines(350, 314, L, "s", 15)
f.save("ch19-linking.svg")

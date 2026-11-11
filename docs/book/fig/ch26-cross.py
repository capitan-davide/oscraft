from svg import Fig
f = Fig(800, 330)
f.box("n", 20, 16, 760, 300); f.text(40, 36, "the build host (x86-64 Linux, say)", "b")
f.box("y", 40, 130, 110, 44, 3); f.text(95, 157, "prog.c", "m c")
f.box("w", 200, 120, 150, 64, 3); f.text(275, 142, "compiler", "b c"); f.text(275, 158, "told the target triple", "s c"); f.text(275, 173, "aarch64-unknown-freebsd", "ms c")
f.box("y", 400, 130, 90, 44, 3); f.text(445, 157, "prog.o", "m c")
f.box("w", 540, 120, 110, 64, 3); f.text(595, 148, "linker", "b c"); f.text(595, 165, "for the target", "s c")
f.box("u", 690, 124, 76, 56, 3); f.text(728, 147, "prog", "m c"); f.text(728, 164, "runs on", "s c"); f.text(728, 176, "the target", "s c")
f.arrow([(150, 152), (198, 152)]); f.arrow([(350, 152), (398, 152)]); f.arrow([(490, 152), (538, 152)]); f.arrow([(650, 152), (688, 152)])
# sysroot
f.box("g", 200, 226, 450, 74, 3); f.text(425, 246, "the sysroot: the target's own files, on the host", "b c")
f.box("w", 215, 256, 130, 34, 2); f.text(280, 270, "usr/include", "ms c"); f.text(280, 283, "headers", "s c")
f.box("w", 360, 256, 130, 34, 2); f.text(425, 270, "usr/lib/crt1.o ...", "ms c"); f.text(425, 283, "start-up objects", "s c")
f.box("w", 505, 256, 130, 34, 2); f.text(570, 270, "libc.a, libc.so", "ms c"); f.text(570, 283, "libraries", "s c")
f.arrow([(275, 224), (275, 186)], "lng", "ag"); f.arrow([(595, 224), (595, 186)], "lng", "ag"); f.text(603, 210, "crt objects, libraries", "s")
f.text(283, 210, "#include", "s"); 
f.text(275, 84, "nothing from the host's own /usr/include or /usr/lib is used", "s c")
f.save("ch26-cross.svg")

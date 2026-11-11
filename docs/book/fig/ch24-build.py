from svg import Fig
f = Fig(800, 330)
f.box("n", 20, 30, 200, 50); f.text(120, 50, "FreeBSD's list of calls", "c"); f.text(120, 66, "sys/sys/syscall.mk", "ms c")
f.box("k", 20, 120, 200, 76); f.text(120, 140, "user/libsys/src/*.rs", "ms c"); f.text(120, 158, "about 200 calls, by hand", "s c"); f.text(120, 176, "stubs.rs: the others, as stubs,", "s c"); f.text(120, 190, "and every alias", "s c")
f.arrow([(120, 80), (120, 118)]); f.text(128, 96, "gen-stubs.py: the calls", "s"); f.text(128, 110, "with no hand-written function", "s")
f.arrow([(220, 158), (300, 158)]); f.text(260, 138, "cargo rustc,", "s c"); f.text(260, 150, "one unit", "s c")
f.box("y", 302, 132, 150, 52, 3); f.text(377, 154, "oscraft_libsys.o", "ms c"); f.text(377, 172, "one relocatable object", "s c")
f.box("g", 302, 30, 150, 60, 3); f.text(377, 50, "FreeBSD's own files", "s c"); f.text(377, 66, "__error.c, auxv.c, ...", "ms c"); f.text(377, 82, "compiled as always", "s c")
f.box("g", 500, 20, 280, 290); f.text(640, 42, "FreeBSD's build, given the object", "b c")
outs = [(60, "libsys.a, libsys.so.7", "the system-call library"), (130, "libc.a", "static programs: the calls are inside"), (200, "ld-elf.so.1", "the dynamic loader's private copy"), (258, "libc.so.7, libthr.so.3", "unchanged: they import from libsys.so")]
for y, a, b in outs:
    f.box("w", 516, y, 248, 44, 3); f.text(640, y + 18, a, "ms c"); f.text(640, y + 34, b, "s c")
for y in (82, 152, 222):
    f.arrow([(452, 158), (476, 158), (476, y), (514, y)], "ln", "a")
f.arrow([(452, 60), (486, 60), (486, 72), (514, 72)], "lng", "ag")
f.text(377, 204, "takes the place of FreeBSD's", "s c"); f.text(377, 218, "generated trap stubs,", "s c"); f.text(377, 232, "cerror and vfork", "s c")
f.save("ch24-build.svg")

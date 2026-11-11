from svg import Fig
f = Fig(800, 440)
# FreeBSD side
f.box("g", 20, 16, 230, 50); f.text(135, 37, "FreeBSD stable/15", "b c"); f.text(135, 54, "at the commit in UPSTREAM_COMMIT", "s c")
f.box("y", 20, 86, 230, 50); f.text(135, 107, "tools/freebsd/patches/", "m c"); f.text(135, 124, "12 commits, kept as patches", "s c")
f.box("g", 300, 46, 200, 60); f.text(400, 68, "the fork, branch oscraft", "b c"); f.text(400, 85, "a checkout beside the", "s c"); f.text(400, 98, "repository: ../freebsd-src", "s c")
f.arrow([(250, 41), (275, 41), (275, 66), (298, 66)]); f.arrow([(250, 111), (275, 111), (275, 86), (298, 86)])
# libsys
f.box("k", 560, 16, 220, 50); f.text(670, 37, "user/libsys  (Rust)", "b c"); f.text(670, 54, "make libsys", "ms c")
f.box("k", 560, 86, 220, 40, 3); f.text(670, 104, "oscraft_libsys.o", "m c"); f.text(670, 118, "every system call, in one object", "s c")
f.arrow([(670, 66), (670, 84)], "lnk", "ak")
# the build
f.box("w", 200, 160, 400, 60); f.text(400, 182, "FreeBSD's own build, cross from the host", "b c"); f.text(400, 199, "make freebsd-libs   make freebsd-progs", "ms c"); f.text(400, 213, "upstream clang and lld, target aarch64-unknown-freebsd", "s c")
f.arrow([(400, 106), (400, 158)]); f.arrow([(670, 126), (670, 144), (540, 144), (540, 158)], "lnk", "ak")
f.text(408, 138, "replaces the trap stubs", "s")
# the sysroot
f.box("g", 60, 256, 680, 86); f.text(400, 276, "the sysroot  (FreeBSD's temporary tree, in ../freebsd-obj)", "b c")
cells = [("usr/include", "headers"), ("usr/lib", "crt objects, libc.a ..."), ("lib", "libc.so.7, libsys.so.7 ..."), ("libexec", "ld-elf.so.1"), ("oscraft/bin", "sh, ls, ps, ifconfig ...")]
for i, (a, b) in enumerate(cells):
    x = 72 + i * 132; w = 126
    f.box("w", x, 288, w, 42, 2); f.text(x + w / 2, 305, a, "ms c"); f.text(x + w / 2, 321, b, "s c" if len(b) < 24 else "s c")
f.arrow([(400, 220), (400, 254)])
# users
outs = [(60, "the boot image", "programs, shared libraries, loader"), (290, "native servers and tests", "compiled and linked against it"), (520, "the SDK", "its headers and libraries, packed")]
for x, t, sub in outs:
    f.box("u", x, 376, 220, 48); f.text(x + 110, 396, t, "b c"); f.text(x + 110, 413, sub, "s c")
    f.arrow([(x + 110, 342), (x + 110, 374)])
f.save("ch26-source-flow.svg")

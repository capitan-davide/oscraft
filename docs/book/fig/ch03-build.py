from svg import Fig
f = Fig(800, 430)
rows = [
 ("k", "kernel/", "Rust, no std", "cargo", "the kernel", "one ELF file: microkernel and process 1"),
 ("k", "startup/", "Rust, no std", "cargo", "startup-qemu-virt", "runs first, describes the machine"),
 ("k", "user/libsys/", "Rust", "cargo", "libsys object", "linked into FreeBSD's libc and loader"),
 ("g", "FreeBSD's source", "the fork, stable/15", "FreeBSD's build", "libc, rtld, sh, utilities", "the sysroot, for aarch64-unknown-freebsd"),
 ("k", "user/ servers", "Rust, with FreeBSD's kernel C", "cargo + clang", "netd, pcid, tmpfs, ...", "ordinary FreeBSD programs"),
 ("g", "FreeBSD drivers", "sys/dev/..., unchanged", "clang", "modules (.ko)", "loaded into netd at run time"),
]
y = 20
for cls, a, asub, tool, b, bsub in rows:
    f.box(cls, 20, y, 200, 44); f.text(120, y + 19, a, "b c"); f.text(120, y + 35, asub, "s c")
    f.arrow([(220, y + 22), (318, y + 22)]); f.text(269, y + 14, tool, "s c")
    f.box("w", 320, y, 250, 44); f.text(445, y + 19, b, "c"); f.text(445, y + 35, bsub, "s c")
    f.line([(570, y + 22), (610, y + 22)])
    y += 56
f.line([(610, 42), (610, y - 34)])
f.arrow([(610, 100), (648, 100)])
f.box("k", 650, 70, 130, 80); f.text(715, 100, "mkifs", "b c"); f.text(715, 117, "tools/mkifs", "s c"); f.text(715, 132, "packs the image", "s c")
f.box("y", 20, y + 6, 550, 40); f.text(295, y + 24, "images/qemu-virt.build", "b c"); f.text(295, y + 39, "the buildfile: which file goes where, and the boot script", "s c")
f.arrow([(570, y + 26), (630, y + 26), (630, 130), (648, 130)])
f.box("y", 650, 220, 130, 76); f.text(715, 244, "build/oscraft.ifs", "b c"); f.text(715, 262, "startup, kernel,", "s c"); f.text(715, 277, "boot script, files", "s c")
f.arrow([(715, 150), (715, 218)])
f.box("n", 650, 356, 130, 60); f.text(715, 380, "make run", "m c"); f.text(715, 398, "QEMU's virt", "s c")
f.arrow([(715, 296), (715, 354)])
f.save("ch03-build.svg")

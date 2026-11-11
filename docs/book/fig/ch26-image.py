from svg import Fig
f = Fig(800, 420)
f.text(170, 24, "where it is built, on the host", "h c"); f.text(590, 24, "where it is, in the boot image", "h c")
rows = [
    ("k", "target/aarch64-unknown-freebsd/release/", "Rust, static: make native", ["/proc/boot/pipe  random  pty  tmpfs", "syslogd  netd  pcid  usbd  diskd-md"]),
    ("g", "<sysroot>/oscraft/bin/", "make freebsd-progs", ["/bin/sh  /bin/ls  /bin/ps  /sbin/ifconfig", "/usr/bin/grep  /usr/bin/top ..."]),
    ("g", "<sysroot>/lib/  and  usr/lib/", "make freebsd-libs, freebsd-progs", ["/lib/libc.so.7  libsys.so.7  libthr.so.3", "libedit.so.8  libtinfow.so.9  libxo.so.0 ..."]),
    ("g", "<sysroot>/libexec/ld-elf.so.1", "make freebsd-libs", ["/libexec/ld-elf.so.1", "the path every dynamic program names"]),
    ("y", "build/freebsd/terminfo/", "make freebsd-terminfo (the host's tic)", ["/usr/share/terminfo/x/xterm", "and seven more terminal types"]),
    ("y", "build/pwd/", "make pwd-db, from images/master.passwd", ["/etc/pwd.db  /etc/spwd.db", "next to /etc/passwd and /etc/group"]),
]
y = 40
for cls, src, how, dst in rows:
    f.box(cls, 20, y, 300, 50, 3); f.text(170, y + 21, src, "ms c"); f.text(170, y + 38, how, "s c")
    f.box("w", 400, y, 380, 50, 3); f.text(590, y + 21, dst[0], "ms c"); f.text(590, y + 38, dst[1], "ms c" if dst[1].startswith(("/", "lib", "sys")) else "s c")
    f.arrow([(320, y + 25), (398, y + 25)])
    y += 60
f.text(360, 30, "mkifs", "s c")
f.text(400, 410, "images/qemu-virt.build names each file once: its path in the image, and the host file it is copied from.", "s c")
f.save("ch26-image.svg")

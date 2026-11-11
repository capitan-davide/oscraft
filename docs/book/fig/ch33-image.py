from svg import Fig
f = Fig(800, 380)
f.text(200, 24, "The image built with QNX=1", "h c")
f.box("g", 20, 40, 360, 150); f.text(200, 60, "the base image (always)", "b c")
f.lines(34, 82, ["/proc/boot/kernel, the startup", "/bin, /sbin, /usr/bin: FreeBSD's sh and utilities", "/lib, /usr/lib: FreeBSD's libc, libthr, ld-elf.so.1",
                 "the native servers: pipe, random, pty, tmpfs,", "syslogd, pcid, netd"], "s", 17)
f.box("r", 20, 200, 360, 150); f.text(200, 220, "the overlay (QNX=1): /compat/qnx", "b c")
f.lines(34, 242, ["lib/ldqnx-64.so.2, libc.so.6, libsocket.so.4, ...", "bin/ksh, bin/sh (a link to ksh), usr/bin/toybox and its links",
                  "bin/pidin, slogger2, slog2info; usr/sbin/sshd, tracelogger", "QNX's servers: mqueue, devc-pty, devc-serpl011, ..."], "s", 17)
f.box("k", 34, 314, 332, 26, 2); f.text(200, 331, "sbin/qnxd: OsCraft's own, the one native program here", "s c")
f.text(200, 368, "Red: files copied from the user's QNX SDP when the image is built.", "s c")
# boot script
f.text(600, 24, "The boot script, in order", "h c")
steps = [("g", "the base image's script", "native servers, the network"),
         ("k", "/compat/qnx/sbin/qnxd", "returns once it has registered"),
         ("r", "/compat/qnx/bin/slogger2", "QNX's logger, for QNX's services"),
         ("r", "/compat/qnx/bin/ksh /etc/rc.net &", "waits for /dev/socket, starts QNX's sshd")]
y = 40
for i, (cls, a, b) in enumerate(steps):
    f.box(cls, 440, y, 320, 48); f.text(600, y + 20, a, "m c"); f.text(600, y + 37, b, "s c")
    if i < len(steps) - 1: f.arrow([(600, y + 48), (600, y + 62)])
    y += 64
f.text(600, y + 12, "Until qnxd has registered, a QNX program", "s c")
f.text(600, y + 27, "cannot use a native server at all.", "s c")
f.save("ch33-image.svg")

from svg import Fig
f = Fig(800, 600)
def row(cls, y, h, title, sub=None, x=20, w=590):
    f.box(cls, x, y, w, h)
    f.text(x + 12, y + 20, title, "h")
    if sub: f.text(x + 12, y + 37, sub, "s")
def cells(y, items, cls="w", x=30, w=570, h=30):
    n = len(items); cw = (w - (n - 1) * 6) / n
    for i, t in enumerate(items):
        cx = x + i * (cw + 6)
        f.box(cls, cx, y, cw, h, 3)
        parts = t.split("|")
        if len(parts) == 1: f.text(cx + cw / 2, y + 19, t, "s c")
        else:
            f.text(cx + cw / 2, y + 13, parts[0], "s c"); f.text(cx + cw / 2, y + 25, parts[1], "s c")
# programs
row("g", 15, 48, "Programs", "FreeBSD's sh, ls, ps, top, ifconfig, ping, kldload, ...  and your own")
# libc
row("g", 71, 70, "FreeBSD's C library, thread library and dynamic loader", "libc, libthr, ld-elf.so: built from FreeBSD's source, unchanged")
f.box("k", 30, 113, 570, 22, 3); f.text(315, 128, "libsys: FreeBSD's system calls, made of OsCraft kernel calls and messages", "s c")
# servers
row("k", 149, 106, "Servers: ordinary processes", None)
cells(175, ["pipe", "pty", "random", "tmpfs", "syslogd", "diskd"])
f.box("wk", 30, 211, 570, 36, 3)
f.text(42, 233, "netd   pcid   usbd", "m")
f.box("g", 190, 216, 400, 26, 3); f.text(390, 233, "FreeBSD's network stack, PCI bus and USB stack, over bsdkern", "s c")
# EL line
f.line([(10, 266), (620, 266)], "lnr", True); f.text(626, 262, "user mode (EL0)", "s red"); f.text(626, 278, "kernel mode (EL1)", "s red")
# process 1
row("k", 277, 68, "Process 1: the managers", None)
cells(303, ["process|manager", "pathname|manager", "memory|manager", "image|file system", "/proc", "consoles,|/dev/null, ..."], "wk", h=34)
# microkernel
row("k", 353, 68, "Microkernel", None)
cells(379, ["threads,|scheduler", "message|passing", "synchron-|ization", "signals", "clocks,|timers", "interrupts", "address|spaces"], "wk", h=34)
f.box("k", 20, 429, 590, 30); f.text(315, 448, "AArch64 layer: exception vectors, kernel entry and exit, context switch, MMU", "c")
f.box("frame", 20, 467, 590, 30); f.text(315, 486, "system page and callouts: the machine, described for the kernel", "c")
f.box("k", 20, 505, 590, 30); f.text(315, 524, "startup (startup-qemu-virt): finds memory, CPUs and devices; starts the kernel", "c")
f.box("n", 20, 543, 590, 40); f.text(315, 560, "QEMU's virt machine", "b c"); f.text(315, 575, "4 Cortex-A72 cores, GICv2, generic timer, PL011 UART, PCIe with a virtio network device", "s c")
# bracket: one image file
f.line([(616, 281), (622, 281), (622, 455), (616, 455)], "lnk"); f.text(630, 360, "one program:", "s"); f.text(630, 374, "the kernel file", "s")
# legend
f.text(640, 30, "Who wrote it", "h")
f.box("k", 640, 44, 18, 14, 2); f.text(666, 56, "OsCraft (Rust)", "s")
f.box("g", 640, 66, 18, 14, 2); f.text(666, 78, "FreeBSD, from", "s"); f.text(666, 91, "its source", "s")
f.box("r", 640, 104, 18, 14, 2); f.text(666, 116, "QNX, optional", "s")
f.box("n", 640, 126, 18, 14, 2); f.text(666, 138, "the machine", "s")
# optional QNX
f.box("r", 632, 160, 158, 90, 4)
f.text(711, 178, "Optional: QNX", "b c")
f.text(711, 195, "QNX programs with", "s c"); f.text(711, 208, "QNX's own libc, under", "s c"); f.text(711, 221, "/compat/qnx; qnxd", "s c"); f.text(711, 234, "translates for them", "s c")
f.save("ch02-system.svg")

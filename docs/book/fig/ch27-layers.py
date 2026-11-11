from svg import Fig
f = Fig(800, 420)
f.box("frame", 20, 14, 590, 330); f.text(32, 32, "one process: netd, pcid or usbd", "s")
rows = [("k", "the server's own code", "its messages: sockets in netd, PCIOCROUTE in pcid, /dev/usbd in usbd"),
        ("g", "FreeBSD's kernel code, not edited", "a subsystem as a library (libbsdnet.a, libbsdpci.a, libbsdusb.a) and modules (.ko)"),
        ("k", "bsdkern", "C in kern/ (compiled as kernel code) and Rust in src/"),
        ("g", "FreeBSD's libc and libthr", "pthread mutexes and threads, mmap(), dlopen(), open(), ioctl()"),
        ("k", "libsys", "each system call a kernel call or a message")]
y = 44
for cls, t, s in rows:
    f.box(cls, 40, y, 550, 48); f.text(315, y + 20, t, "b c"); f.text(315, y + 37, s, "s c"); y += 58
f.box("k", 20, 360, 590, 44); f.text(315, 379, "the microkernel and process 1", "b c"); f.text(315, 395, "threads, messages, interrupts, memory mappings", "s c")
for yy in (92, 150, 208, 266): f.arrow([(315, yy), (315, yy + 10)])
f.arrow([(315, 324), (315, 358)])
notes = [(155, ["FreeBSD's kernel interfaces:", "mtx_lock(), malloc(9),", "msleep(), callout_reset(),", "bus_alloc_resource() ..."]),
         (213, ["POSIX: what any", "FreeBSD program uses"]), (342, ["kernel calls: MsgSendv(),", "InterruptWait() ..."])]
for yy, ls in notes:
    f.line([(596, yy), (626, yy)], "life"); f.lines(632, yy + 4 - (len(ls) - 1) * 7, ls, "s")
f.save("ch27-layers.svg")

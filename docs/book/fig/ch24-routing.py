from svg import Fig
f = Fig(800, 500)
f.box("k", 20, 20, 330, 460); f.text(185, 42, "libsys: FreeBSD's system calls", "h c")
rows = [
    ("threads, locks, signals, clocks", "thr_new, _umtx_op, sigaction, kill,", "clock_gettime, nanosleep, ktimer_*"),
    ("waiting for several descriptors", "poll, select, kevent", ""),
    ("an open descriptor", "read, write, lseek, fstat, ioctl,", "fcntl, getdirentries, close"),
    ("sockets", "socket, bind, connect, accept,", "sendmsg, recvmsg, setsockopt"),
    ("path names", "open, stat, unlink, mkdir,", "rename, chdir, readlink"),
    ("memory", "mmap, munmap, mprotect", ""),
    ("processes and identity", "fork, execve, wait4, getuid,", "setsid, setpgid, umask"),
]
ys = []
y = 54
for title, a, b in rows:
    f.box("wk", 34, y, 302, 50, 3); f.text(44, y + 16, title, "b"); f.text(44, y + 31, a, "ms"); f.text(44, y + 44, b, "ms")
    ys.append(y + 25); y += 54
f.box("w", 34, y, 302, 40, 3); f.text(44, y + 16, "answered in libsys, or nothing to do", "b"); f.text(44, y + 31, "getpid, hw.ncpu, mlock, msync", "ms")
# destinations
f.box("k", 520, 30, 260, 92); f.text(650, 52, "the kernel", "h c"); f.text(650, 72, "kernel calls: ThreadDestroy(),", "s c"); f.text(650, 87, "SignalAction(), ClockTime(),", "s c"); f.text(650, 102, "TimerTimeout(), thr_new, futex", "s c")
f.box("u", 520, 152, 260, 122); f.text(650, 174, "servers", "h c"); f.text(650, 194, "tmpfs, pipe, pty, random, diskd,", "s c"); f.text(650, 209, "netd (sockets), pcid, usbd,", "s c"); f.text(650, 224, "process 1's own devices and files", "s c")
f.text(650, 248, "a message on the descriptor", "s c"); f.text(650, 263, "MsgSendv(fd, ...)", "ms c")
f.box("k", 520, 304, 260, 122); f.text(650, 326, "process 1", "h c"); f.text(650, 346, "the pathname manager,", "s c"); f.text(650, 361, "the process manager,", "s c"); f.text(650, 376, "the memory manager", "s c")
f.text(650, 400, "a message on the connection", "s c"); f.text(650, 415, "every process is born with", "s c")
def go(i, to, cls="lnk", m="ak"): f.arrow([(336, ys[i]), (518, to)], cls, m)
go(0, 66); go(1, 96); go(1, 176, "ln", "a"); go(2, 200, "ln", "a"); go(3, 224, "ln", "a"); go(4, 250, "ln", "a")
go(4, 330); go(5, 360); go(6, 390)
f.text(650, 452, "blue arrows: kernel calls and process 1", "s c"); f.text(650, 467, "black arrows: other servers", "s c")
f.save("ch24-routing.svg")

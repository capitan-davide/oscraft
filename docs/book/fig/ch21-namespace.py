from svg import Fig
rows = [
    (0, "/", "the boot image's files: /bin, /lib, /etc ...", "process 1", "k"),
    (1, "proc/", "processes as files", "process 1", "k"),
    (1, "dev/", "attached by nobody: it exists because names go on below it", "", "w"),
    (2, "text, console", "the console", "process 1", "k"),
    (2, "null, zero, klog, tty", "", "process 1", "k"),
    (2, "shmem/, sem/", "shared memory objects, named semaphores", "process 1", "k"),
    (2, "pipe", "", "pipe", "u"),
    (2, "random, urandom", "", "random", "u"),
    (2, "ptyp0 ... ptyp7, ttyp0 ... ttyp7", "", "pty", "u"),
    (2, "socket/2, /28, /1, /17, /38", "one name per socket domain", "netd", "g"),
    (2, "netd", "loading modules into the network server", "netd", "g"),
    (2, "pci", "", "pcid", "g"),
    (1, "tmp/", "a file system in memory", "tmpfs", "u"),
    (1, "var/", "another", "tmpfs", "u"),
    (2, "run/log", "the logger's name, inside tmpfs's /var", "syslogd", "u"),
]
H = 26
f = Fig(820, 62 + len(rows) * H + 54)
f.text(30, 26, "name", "b"); f.text(690, 26, "attached by", "b")
f.line([(20, 36), (800, 36)])
for i, (ind, name, what, owner, cls) in enumerate(rows):
    y = 46 + i * H
    x = 30 + ind * 26
    f.text(x, y + 16, name, "m")
    if what:
        f.text(680, y + 16, what, "s e")
    if owner:
        f.box(cls, 690, y + 1, 110, H - 6, 3); f.text(745, y + 15, owner, "s c")
    if ind:
        f.line([(x - 16, y - 6), (x - 16, y + 12), (x - 5, y + 12)], "life")
y = 46 + len(rows) * H + 8
f.line([(20, y), (800, y)])
f.text(30, y + 20, "by file type, at any path", "b")
f.text(260, y + 20, "a pipe() goes to pipe; shm_open() and sem_open() go to process 1", "s")
f.box("k", 30, y + 32, 14, 12, 2); f.text(50, y + 43, "process 1", "s")
f.box("u", 130, y + 32, 14, 12, 2); f.text(150, y + 43, "a native server", "s")
f.box("g", 260, y + 32, 14, 12, 2); f.text(280, y + 43, "a server running FreeBSD kernel code", "s")
f.save("ch21-namespace.svg")

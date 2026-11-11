from svg import Fig
f = Fig(800, 330)
rows = [("pipe", 0, "k"), ("random", 0, "k"), ("pty", 0, "k"), ("tmpfs", 1, "k"), ("syslogd", 2, "k"), ("netd", 3, "g"), ("pcid", 4, "g"), ("sh", 5, "u")]
gx = [150, 262, 374, 486, 598, 710]
gates = [(1, ["/dev/pipe"]), (2, ["/var"]), (3, ["/var/run/log", "/dev/urandom", "/dev/ptyp0"]), (4, ["/dev/netd"]), (5, ["/dev/pci"])]
top = 78
for i, (name, g, cls) in enumerate(rows):
    y = top + i * 28
    f.text(130, y + 15, name, "m e")
    f.box(cls, gx[g] + 6, y, 770 - gx[g] - 6, 20, 3)
for g, names in gates:
    x = gx[g]
    f.line([(x, 60), (x, top + len(rows) * 28)], "lnr", True)
    for j, n in enumerate(names):
        f.text(x, 52 - (len(names) - 1 - j) * 13, n, "ms c")
f.text(130, 52, "the script waits for", "s e")
f.arrow([(150, 316), (770, 316)]); f.text(130, 320, "time", "s e")
f.text(gx[2] + 12, top + 3 * 28 - 6, "", "s")
f.save("ch25-boot-order.svg")

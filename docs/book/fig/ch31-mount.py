from svg import Fig
f = Fig(800, 520)
cols = [(90, "u", "mount", "FreeBSD's mount(8)"), (270, "g", "libsys", "in mount's process"), (470, "k", "diskd-md", "serves /dev/md1"), (680, "k", "process 1", "pathname manager")]
for x, c, t, s in cols:
    f.box(c, x - 75, 14, 150, 36); f.text(x, 30, t, "b c"); f.text(x, 43, s, "s c"); f.line([(x, 50), (x, 440)], "life")
X = [c[0] for c in cols]
def msg(y, a, b, t, n, cls="lnk", m="ak", col="#2c5d9b", dash=False):
    f.arrow([(X[a], y), (X[b], y)], cls, m, dash)
    lo = min(X[a], X[b])
    f.num(lo + 14, y - 12, n, col); f.text(lo + 28, y - 8, t, "s")
f.text(20, 72, "mount -t msdosfs /dev/md1 /tmp/mnt", "ms")
msg(104, 0, 1, "nmount(fstype, from, fspath)", 1)
msg(140, 1, 2, "open /dev/md1", 2)
msg(176, 1, 2, "devctl: mount at /tmp/mnt", 3)
f.box("y", X[2] - 95, 186, 190, 62, 3); f.lines(X[2], 204, ["is block 0 a FAT boot sector?", "the cache goes to a Volume;", "the device is busy from now on"], "s c", 15)
msg(288, 2, 3, "attach the path /tmp/mnt", 4)
f.box("y", X[2] - 95, 298, 190, 30, 3); f.text(X[2], 317, "add a line to /var/run/mounts", "s c")
msg(356, 2, 1, "reply: 0", 5, "lng", "ag", "#3e7b34")
msg(392, 1, 0, "returns 0", 6, "lng", "ag", "#3e7b34")
f.line([(20, 452), (780, 452)], "life")
f.text(400, 474, "Afterwards, open(\"/tmp/mnt/note.txt\") resolves to diskd-md like any path: the pathname manager", "s c")
f.text(400, 490, "names the server with the longest matching prefix, and the file's messages go to it directly.", "s c")
f.save("ch31-mount.svg")

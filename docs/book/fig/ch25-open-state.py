from svg import Fig
f = Fig(800, 330)
# clients
def proc(y, name, fds):
    f.box("u", 20, y, 170, 40 + 26 * len(fds)); f.text(105, y + 20, name, "b c")
    for i, t in enumerate(fds):
        f.box("wu", 40, y + 32 + i * 26, 130, 22, 2); f.text(105, y + 47 + i * 26, t, "m c")
proc(40, "program A", ["fd 3", "fd 4 = dup(3)"])
proc(200, "program B", ["fd 5"])
# the server
f.box("u", 270, 16, 510, 296); f.text(525, 36, "server", "b c")
f.text(390, 62, "one record per open", "s c")
f.box("y", 300, 72, 180, 78, 3); f.text(390, 92, "open file", "b c"); f.text(390, 110, "opened read-only", "s c"); f.text(390, 125, "position 120", "s c"); f.text(390, 140, "used by 2 descriptors", "s c")
f.box("y", 300, 196, 180, 78, 3); f.text(390, 216, "open file", "b c"); f.text(390, 234, "opened read-write", "s c"); f.text(390, 249, "position 0", "s c"); f.text(390, 264, "used by 1 descriptor", "s c")
f.text(655, 100, "one record per file", "s c")
f.box("g", 560, 110, 190, 126, 3); f.text(655, 132, "file  /tmp/notes", "b c")
f.lines(655, 152, ["size 4096", "owner, group, mode", "times", "open 2 times: 2 readers,", "1 writer"], "s c", 15)
# descriptors -> open files
f.arrow([(170, 83), (298, 100)], "lnk", "ak")
f.arrow([(170, 109), (298, 120)], "lnk", "ak")
f.arrow([(170, 243), (298, 235)], "lnk", "ak")
# open files -> file
f.arrow([(480, 111), (558, 150)])
f.arrow([(480, 235), (558, 196)])
f.text(105, 296, "a descriptor is a connection", "s c")
f.save("ch25-open-state.svg")

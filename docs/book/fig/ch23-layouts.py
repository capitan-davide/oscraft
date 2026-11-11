from svg import Fig
f = Fig(800, 400)
B = 26  # pixels per byte, 16 bytes per row
X0, Y0, H = 56, 44, 26
f.text(X0, 28, "struct stat: 224 bytes", "b")
rows = [
    [("st_dev", 8), ("st_ino", 8)],
    [("st_nlink", 8), ("st_mode", 2), ("bsdflags", 2), ("st_uid", 4)],
    [("st_gid", 4), ("padding", 4), ("st_rdev", 8)],
    [("st_atim: seconds, nanoseconds", 16)],
    [("st_mtim", 16)],
    [("st_ctim", 16)],
    [("st_birthtim", 16)],
    [("st_size", 8), ("st_blocks", 8)],
    [("st_blksize", 4), ("st_flags", 4), ("st_gen", 8)],
    [("st_filerev", 8), ("st_spare ...", 8)],
]
y = Y0
for i, r in enumerate(rows):
    f.text(X0 - 8, y + 17, str(i * 16), "ms e")
    x = X0
    for name, n in r:
        cls = "w" if name in ("padding", "st_spare ...") else ("y" if name == "st_size" else "g")
        f.box(cls, x, y, n * B, H, 0); f.text(x + n * B / 2, y + 17, name, "ms c"); x += n * B
    y += H
f.text(X0 - 8, y + 17, "160", "ms e")
f.box("w", X0, y, 16 * B, H + 8, 0); f.text(X0 + 8 * B, y + 21, "st_spare, to byte 224", "ms c")
f.text(X0 + 16 * B / 2, y + 62, "16 bytes in a row; the number at the left is the row's offset", "s c")
# dirent
C = 34; X1 = 500
f.text(X1, 28, "struct dirent: one record of a directory", "b")
drows = [[("d_fileno", 8)], [("d_off", 8)], [("d_reclen", 2), ("d_type", 2), ("d_namlen", 2), ("padding", 2)]]
y = Y0
for i, r in enumerate(drows):
    f.text(X1 - 8, y + 17, str(i * 8), "ms e")
    x = X1
    for name, n in r:
        f.box("w" if name == "padding" else "g", x, y, n * C, H, 0); f.text(x + n * C / 2, y + 17, name, "ms c"); x += n * C
    y += H
f.text(X1 - 8, y + 17, "24", "ms e")
f.box("g", X1, y, 8 * C, H, 0); f.text(X1 + 4 * C, y + 17, "the name, a NUL, padding to 8", "ms c")
f.text(X1 + 4 * C, y + 44, "d_type is one byte, then one of padding", "s c")
# termios
y = 210
f.text(X1, y, "struct termios: 44 bytes", "b")
y += 14
trows = [[("c_iflag", 4), ("c_oflag", 4)], [("c_cflag", 4), ("c_lflag", 4)]]
off = 0
for r in trows:
    f.text(X1 - 8, y + 17, str(off), "ms e"); x = X1
    for name, n in r:
        f.box("g", x, y, n * C, H, 0); f.text(x + n * C / 2, y + 17, name, "ms c"); x += n * C
    y += H; off += 8
f.text(X1 - 8, y + 17, "16", "ms e")
f.box("g", X1, y, 8 * C, H + 8, 0); f.text(X1 + 4 * C, y + 21, "c_cc: 20 control characters", "ms c")
y += H + 8
f.text(X1 - 8, y + 17, "36", "ms e")
f.box("g", X1, y, 4 * C, H, 0); f.text(X1 + 2 * C, y + 17, "c_ispeed", "ms c")
f.box("g", X1 + 4 * C, y, 4 * C, H, 0); f.text(X1 + 6 * C, y + 17, "c_ospeed", "ms c")
f.save("ch23-layouts.svg")

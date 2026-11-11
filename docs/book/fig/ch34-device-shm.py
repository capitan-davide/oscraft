from svg import Fig
f = Fig(800, 320)
f.text(180, 24, "Physical addresses on QEMU's virt", "h c")
X, W = 40, 280
rows = [("0x0900_0000", "n", "the PL011 serial port"), ("0x0901_0000", "g", "the PL031 real-time clock"), ("...", "w", "other devices"), ("0x0910_0000", "r", "nothing answers here")]
y = 44
ys = {}
for a, cls, t in rows:
    f.box(cls, X + 100, y, W - 100, 44, 2); f.text(X + 100 + (W - 100) / 2, y + 27, t, "s c"); f.text(X + 92, y + 27, a, "ms e")
    ys[a] = y + 22; y += 52
PX = 420
f.box("u", PX, 44, 360, 96); f.text(PX + 180, 64, "a memory object placed over the clock", "b c")
f.text(PX + 180, 84, "read() of 4 bytes, twice, 1.2 s apart:", "s c"); f.text(PX + 180, 100, "two different values", "ms c")
f.text(PX + 180, 120, "mmap() and a load from the page: the same", "s c")
f.arrow([(PX, 92), (X + W + 2, ys["0x0901_0000"])], "lng", "ag")
f.box("u", PX, 170, 360, 96); f.text(PX + 180, 190, "a memory object placed where no device is", "b c")
f.text(PX + 180, 210, "write() of 4 bytes:", "s c"); f.text(PX + 180, 226, "-1, ESRVRFAULT", "ms c")
f.text(PX + 180, 246, "and the system carries on", "s c")
f.arrow([(PX, 218), (X + W + 2, ys["0x0910_0000"])], "lnr", "ar")
f.text(400, 300, "read() and write() are requests to process 1, which touches the device's memory itself: a fault there is reported as the server's.", "s c")
f.save("ch34-device-shm.svg")

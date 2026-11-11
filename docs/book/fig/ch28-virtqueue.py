from svg import Fig
f = Fig(800, 400)
f.box("u", 20, 20, 150, 360); f.text(95, 40, "the guest's driver", "h c")
f.box("n", 630, 20, 150, 360); f.text(705, 40, "the device (host)", "h c")
f.box("frame", 190, 20, 420, 360); f.text(400, 40, "the guest's memory, which the host can also reach", "s c")
# descriptor table
f.box("y", 206, 56, 250, 132, 3); f.text(331, 74, "descriptor table", "b c")
cols = [218, 244, 330, 374, 424]
for x, t in zip(cols, ["#", "address", "length", "flags", "next"]): f.text(x, 94, t, "s")
rows = [("0", "of buffer A", "64", "NEXT", "1"), ("1", "of buffer B", "2048", "-", "-"), ("2", "of buffer C", "2048", "-", "-"), ("3", "...", "", "", "")]
for i, r in enumerate(rows):
    for x, t in zip(cols, r): f.text(x, 112 + i * 16, t, "ms")
f.text(331, 180, "addresses are guest-physical", "s c")
# buffers
f.box("y", 480, 56, 116, 132, 3); f.text(538, 74, "buffers", "b c")
f.box("w", 492, 84, 92, 22, 2); f.text(538, 99, "A", "s c")
f.box("w", 492, 112, 92, 30, 2); f.text(538, 131, "B", "s c")
f.box("w", 492, 148, 92, 30, 2); f.text(538, 167, "C", "s c")
f.arrow([(456, 108), (490, 97)], "ln", "a"); f.arrow([(456, 124), (490, 127)], "ln", "a"); f.arrow([(456, 140), (490, 160)], "ln", "a")
# available ring
f.box("y", 206, 206, 390, 60, 3); f.text(216, 224, "available ring", "b"); f.text(330, 224, "written by the driver, read by the device", "s")
f.text(216, 248, "idx = 2", "m")
for i, t in enumerate(["0", "2", "", "", "", ""]):
    f.box("w", 300 + i * 46, 233, 46, 22, 0); f.text(323 + i * 46, 248, t, "ms c")
# used ring
f.box("y", 206, 286, 390, 76, 3); f.text(216, 304, "used ring", "b"); f.text(330, 304, "written by the device, read by the driver", "s")
f.text(216, 330, "idx = 1", "m")
for i, t in enumerate(["0, 78", "", "", "", "", ""]):
    f.box("w", 300 + i * 46, 315, 46, 22, 0); f.text(323 + i * 46, 330, t, "ms c")
f.text(400, 354, "each entry: a chain's first descriptor, and how many bytes were written", "s c")
# driver side
f.box("wu", 32, 216, 126, 40, 3); f.text(95, 233, "offers the chains", "s c"); f.text(95, 247, "starting at 0 and 2", "s c")
f.arrow([(158, 236), (204, 236)], "lnk", "ak"); f.num(180, 222, 1, "#2c5d9b")
f.box("wu", 32, 304, 126, 40, 3); f.text(95, 321, "takes the data,", "s c"); f.text(95, 335, "offers the chain again", "s c")
f.arrow([(204, 324), (160, 324)], "lng", "ag"); f.num(180, 310, 4, "#3e7b34")
f.text(95, 80, "after 1, a notify:", "s c"); f.text(95, 94, "one register write", "s c")
f.text(95, 130, "after 3, an interrupt", "s c"); f.text(95, 144, "from the device", "s c")
# device side
f.box("w", 642, 100, 126, 54, 3); f.text(705, 117, "data arrives:", "s c"); f.text(705, 131, "writes 78 bytes", "s c"); f.text(705, 145, "into chain 0", "s c")
f.arrow([(596, 236), (685, 236), (685, 156)], "lnk", "ak"); f.num(620, 222, 2, "#2c5d9b")
f.arrow([(725, 154), (725, 324), (598, 324)], "lng", "ag"); f.num(620, 310, 3, "#3e7b34")
f.text(705, 350, "reads and writes the", "s c"); f.text(705, 364, "guest's memory directly", "s c")
f.save("ch28-virtqueue.svg")

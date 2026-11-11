from svg import Fig
f = Fig(800, 400)
f.text(20, 24, "a function's configuration space: the first 64 bytes", "h")
rows = [("0x00", [("device ID", 2, "y"), ("vendor ID", 2, "y")]), ("0x04", [("status", 2, "w"), ("command", 2, "k")]),
        ("0x08", [("class, subclass, interface", 3, "y"), ("revision", 1, "w")]), ("0x0c", [("", 1, "w"), ("header type", 1, "w"), ("", 2, "w")]),
        ("0x10", [("BAR 0", 4, "g")]), ("0x14", [("BAR 1", 4, "g")]), ("...", [("BAR 2 to BAR 5", 4, "g")]),
        ("0x2c", [("subsystem ID", 2, "w"), ("subsystem vendor", 2, "w")]), ("0x34", [("", 3, "w"), ("capabilities", 1, "r")]),
        ("0x3c", [("", 2, "w"), ("interrupt pin", 1, "k"), ("line", 1, "w")])]
y = 40
for off, cells in rows:
    f.text(62, y + 19, off, "ms e"); x = 70
    for t, n, cls in cells:
        f.box(cls, x, y, n * 85, 28, 0); f.text(x + n * 85 / 2, y + 18, t, "s c"); x += n * 85
    y += 30
f.text(70, 356, "byte 3", "s"); f.text(410, 356, "byte 0", "s e")
notes = [(54, "y", "who the function is: a driver matches on these"), (84, "k", "command: decode memory, act as bus master"),
         (176, "g", "BARs: where the function's registers are on the bus"), (326, "k", "which INTx pin it uses, A to D")]
for yy, cls, t in notes:
    f.box(cls, 430, yy - 10, 12, 12, 2); f.text(450, yy, t, "s")
f.text(430, 216, "Sizing a BAR: write all ones, read back.", "s"); f.text(430, 231, "The bits that stay 0 give the size.", "s")
f.arrow([(410, 294), (470, 294), (470, 262)], "lnr", "ar")
caps = [("MSI", "0x05"), ("vendor", "0x09"), ("MSI-X", "0x11")]
for i, (n, i_d) in enumerate(caps):
    x = 440 + i * 118
    f.box("r", x, 240, 96, 22, 3); f.text(x + 48, 255, f"{n}  {i_d}", "ms c")
    if i < 2: f.arrow([(x + 96, 251), (x + 118, 251)], "lnr", "ar")
f.text(780, 280, "the capability list: each entry has an ID and", "s e"); f.text(780, 295, "the offset of the next; 0 ends the list", "s e")
f.text(20, 384, "PCI Express extends each function's space from 256 to 4096 bytes.", "s")
f.save("ch29-config-space.svg")

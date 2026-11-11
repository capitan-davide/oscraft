from svg import Fig
f = Fig(800, 330)
def code(x, y, w, title, lines, cls="wk", hl=()):
    h = 26 + 14 * len(lines)
    f.box(cls, x, y, w, h, 3); f.text(x + 10, y + 17, title, "s")
    for i, t in enumerate(lines):
        f.text(x + 10, y + 36 + i * 14, t, "ms red" if i in hl else "ms")
    return h
f.text(130, 22, "In the startup program", "h c")
src = ["movz x7, #0", "movk x7, #0, lsl #16", "movk x7, #0, lsl #32", "movk x7, #0, lsl #48", "ldr  w2, [x7, #0x18]", "...", "str  w0, [x7]", "ret"]
code(20, 34, 220, "display_char, as written:", src, "wk", (0, 1, 2, 3))
f.text(130, 196, "The first four instructions load", "s c"); f.text(130, 210, "a 64-bit constant, still zero:", "s c"); f.text(130, 224, "the UART's address is not known", "s c"); f.text(130, 238, "until startup has mapped it.", "s c")
f.arrow([(242, 100), (288, 100)], "ln", "a"); f.text(265, 88, "copy,", "s c"); f.text(265, 122, "patch", "s c")
f.text(410, 22, "In the system page", "h c")
dst = ["movz x7, #<bits 0-15>", "movk x7, #<bits 16-31>, lsl #16", "movk x7, #<bits 32-47>, lsl #32", "movk x7, #<bits 48-63>, lsl #48", "ldr  w2, [x7, #0x18]", "...", "str  w0, [x7]", "ret"]
f.box("y", 290, 34, 250, 240); 
code(300, 44, 230, "the copy, patched:", dst, "w", (0, 1, 2, 3))
f.box("w", 300, 200, 230, 64, 3); f.text(310, 217, "the callout section:", "s"); f.text(310, 235, "debug[0].display_char =", "ms"); f.text(310, 250, "    address of the copy", "ms")
f.arrow([(415, 200), (415, 186)], "ln", "a")
f.text(415, 292, "x7 = where startup mapped the PL011's", "s c"); f.text(415, 306, "registers in the kernel's address space", "s c")
f.text(680, 22, "In the kernel", "h c")
f.box("k", 580, 60, 200, 60); f.text(680, 82, "kprintln!(...)", "m c"); f.text(680, 102, "for each byte: putc()", "s c")
f.box("k", 580, 150, 200, 76); f.text(680, 172, "putc()", "m c"); f.text(680, 190, "reads the pointer from the", "s c"); f.text(680, 204, "system page and calls it:", "s c"); f.text(680, 218, "display_char(syspage, byte)", "ms c")
f.arrow([(680, 120), (680, 148)], "lnk", "ak")
f.arrow([(578, 188), (542, 188), (542, 110), (532, 110)], "lnk", "ak")
f.text(680, 256, "The kernel contains no UART driver", "s c"); f.text(680, 270, "and no UART address.", "s c")
f.save("ch05-callout.svg")

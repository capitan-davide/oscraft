from svg import Fig
f = Fig(800, 360)
f.text(60, 26, "the caller's code", "h")
rows = [("A", "wu", "mov   w8, #number"), ("A+4", "u", "svc   #0x51"), ("A+8", "g", "success continues here"), ("A+12", "r", "failure continues here")]
for i, (a, c, t) in enumerate(rows):
    y = 40 + i * 44
    f.box(c, 60, y, 240, 34, 3); f.text(52, y + 21, a, "ms e"); f.text(74, y + 21, t, "m" if i < 2 else "")
f.line([(478, 16), (478, 222)], "lnr", True); f.text(470, 28, "the caller", "s red e"); f.text(486, 28, "the kernel", "s red")
f.box("k", 500, 62, 270, 150)
f.text(635, 86, "ker::dispatch()", "c")
f.text(635, 106, "the handler returns a value or an error;", "s c")
f.text(635, 122, "complete_kercall() writes the saved registers:", "s c")
f.text(545, 150, "Ok(v):   x0 = v", "m"); f.text(545, 170, "Err(e):  x0 = e,  elr += 4", "m")
f.arrow([(300, 101), (500, 101)]); f.text(388, 93, "svc: return address A+8", "s c")
f.arrow([(500, 145), (302, 145)], "lng", "ag"); f.text(388, 138, "return to elr", "s c green")
f.arrow([(500, 189), (302, 189)], "lnr", "ar"); f.text(388, 182, "return to elr + 4", "s c red")
# the three stubs
f.text(60, 252, "what the three kinds of caller put at the two return points", "h")
stubs = [("process 1's threads", "ret", "neg  x0, x0", "an error becomes a negative result"),
         ("native servers", "ret", "b    <error routine>", "which sets errno and returns -1"),
         ("libsys", "b    1f", "mov  x9, #1", "a flag the Rust code tests")]
for i, (t, a, b, n) in enumerate(stubs):
    x = 60 + i * 240
    f.box("w", x, 264, 228, 84, 3)
    f.text(x + 12, 282, t, "b")
    f.text(x + 12, 302, "A+8", "ms"); f.text(x + 52, 302, a, "m")
    f.text(x + 12, 320, "A+12", "ms"); f.text(x + 52, 320, b, "m")
    f.text(x + 12, 338, n, "s")
f.save("ch07-return-points.svg")

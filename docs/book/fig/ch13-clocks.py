from svg import Fig
f = Fig(800, 360)
OX, OY = 90, 300          # origin
f.arrow([(OX, OY), (760, OY)]); f.text(760, OY + 18, "real time", "s e")
f.arrow([(OX, OY), (OX, 30)]); f.text(OX - 8, 36, "clock", "s e"); f.text(OX - 8, 49, "reads", "s e")
f.text(OX, OY + 18, "boot", "s c")
# monotonic: slope
def mono(x): return OY - (x - OX) * 0.22
f.line([(OX, mono(OX)), (740, mono(740))], "lnk")
f.text(748, mono(740) + 4, "", "s")
f.text(560, mono(560) + 32, "monotonic", "b c blue"); f.text(560, mono(560) + 46, "0 at boot, never set", "s c")
# realtime: mono + offset, with a step back at XS
OFF, XS, STEPB = 120, 430, 50
def real1(x): return mono(x) - OFF
def real2(x): return mono(x) - OFF + STEPB
f.line([(OX, real1(OX)), (XS, real1(XS))], "lng")
f.line([(XS, real1(XS)), (XS, real2(XS))], "lnr", True)
f.line([(XS, real2(XS)), (740, real2(740))], "lng")
f.text(250, real1(250) - 30, "realtime (time of day)", "b c green")
f.text(XS + 10, real1(XS) + 6, "the time of day is set:", "s red"); f.text(XS + 10, real1(XS) + 20, "the clock jumps (here, back)", "s red")
# offset brace
xb = 160
f.arrow([(xb, mono(xb) - 3), (xb, real1(xb) + 3)], "ln", "a", False, True)
f.text(xb + 8, (mono(xb) + real1(xb)) / 2 + 4, "offset: the time of day at boot", "s")
xb2 = 670
f.arrow([(xb2, mono(xb2) - 3), (xb2, real2(xb2) + 3)], "ln", "a", False, True)
f.text(xb2 + 8, (mono(xb2) + real2(xb2)) / 2 + 4, "new offset", "s")
# an interval measured across the step
xa, xc = 370, 520
for x in (xa, xc):
    f.line([(x, OY - 4), (x, OY + 4)])
f.arrow([(xa, OY - 14), (xc, OY - 14)], "ln", "a", False, True)
f.text((xa + xc) / 2, OY - 20, "an interval measured across the jump", "s c")
f.text(400, 348, "Only the monotonic clock gives the interval's true length; the realtime clock would report it too short.", "s c")
f.save("ch13-clocks.svg")

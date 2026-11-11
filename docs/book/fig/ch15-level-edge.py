from svg import Fig
f = Fig(800, 400)
def wave(y, segs, cls="ln"):
    # segs: list of (x, level) ; level 0/1 ; draws step waveform
    pts = []
    lo, hi = y + 22, y
    cur = segs[0][1]
    pts.append((segs[0][0], hi if cur else lo))
    for x, lv in segs[1:]:
        pts.append((x, hi if cur else lo))
        if lv != cur:
            pts.append((x, hi if lv else lo))
        cur = lv
    f.line(pts, cls)
X0, X1 = 170, 770
def panel(y0, title):
    f.text(20, y0, title, "h")
    f.text(160, y0 + 40, "the device's line", "s e")
    f.text(160, y0 + 84, "at the controller", "s e")
    f.text(160, y0 + 128, "software", "s e")
# edge
panel(24, "Edge-triggered: the event is the change")
wave(46, [(X0, 0), (260, 1), (285, 0), (520, 1), (545, 0), (X1, 0)])
f.box("y", 260, 94, 110, 24, 3); f.text(315, 110, "pending", "s c")
f.box("y", 520, 94, 110, 24, 3); f.text(575, 110, "pending", "s c")
f.box("g", 370, 138, 90, 24, 3); f.text(415, 154, "handler", "s c")
f.box("g", 630, 138, 90, 24, 3); f.text(675, 154, "handler", "s c")
f.line([(260, 46), (260, 94)], "life"); f.line([(520, 46), (520, 94)], "life")
f.text(480, 86, "the controller remembers the pulse; one interrupt per pulse", "s c")
# level
y = 214
panel(y, "Level-triggered: the line stays active until the device is served")
wave(y + 22, [(X0, 0), (260, 1), (560, 0), (X1, 0)])
f.box("g", X0, y + 70, 110, 24, 3); f.text(X0 + 55, y + 86, "unmasked", "s c")
f.box("r", 280, y + 70, 320, 24, 3); f.text(440, y + 86, "masked: the active line reaches no CPU", "s c")
f.box("g", 600, y + 70, 170, 24, 3); f.text(685, y + 86, "unmasked", "s c")
f.box("k", 260, y + 114, 60, 24, 3); f.text(290, y + 130, "handler", "s c")
f.box("u", 420, y + 114, 140, 24, 3); f.text(490, y + 130, "driver serves the device", "s c")
f.line([(260, y + 22), (260, y + 114)], "life")
f.line([(560, y + 22), (560, y + 114)], "life")
f.arrow([(600, y + 150), (600, y + 96)], "lng", "ag"); f.text(608, y + 150, "safe to unmask: the line is quiet", "s green")
f.text(20, y + 172, "Unmasked while the line is still active, a level-triggered interrupt is taken again at once, before any driver has run.", "s")
f.save("ch15-level-edge.svg")

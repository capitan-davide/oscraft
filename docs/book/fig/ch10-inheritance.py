from svg import Fig
f = Fig(800, 410)
def panel(y0, title, bars, notes):
    f.text(20, y0, title, "h")
    lanes = [("client H, priority 30", y0 + 28), ("thread M, priority 20", y0 + 62), ("server S, priority 10", y0 + 96)]
    for name, y in lanes:
        f.text(170, y + 14, name, "e")
        f.line([(180, y + 22), (770, y + 22)], "life")
    for lane, x0, x1, cls, label in bars:
        y = lanes[lane][1]
        f.box(cls, x0, y, x1 - x0, 22, 2)
        if label: f.text((x0 + x1) / 2, y + 15, label, "s c")
    for x, y, t, cls in notes:
        f.text(x, y, t, cls)
    # time axis
    f.arrow([(180, y0 + 132), (770, y0 + 132)]); f.text(770, y0 + 147, "time", "s e")
# Without inheritance
y = 30
panel(y, "Without inheritance: priority inversion", [
    (2, 180, 260, "g", "runs"),
    (0, 260, 300, "g", "sends"),
    (0, 300, 770, "r", "waits for S, which cannot run: as long as M likes"),
    (2, 300, 350, "g", "runs"),
    (1, 350, 770, "g", "M is ready, and 20 beats 10: M runs"),
    (2, 350, 770, "n", "S is preempted, holding H's request"),
], [])
y = 240
panel(y, "With inheritance: the server runs at its client's priority", [
    (2, 180, 260, "g", "runs"),
    (0, 260, 300, "g", "sends"),
    (0, 300, 470, "y", "waits for S"),
    (2, 300, 470, "k", "runs at 30: H's request"),
    (1, 350, 620, "n", "M is ready, waits its turn"),
    (0, 470, 620, "g", "has its reply, runs"),
    (1, 620, 770, "g", "M runs"),
    (2, 470, 770, "n", "replied; waits to receive the next message"),
], [])
f.save("ch10-inheritance.svg")

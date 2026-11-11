from svg import Fig
f = Fig(800, 330)
W = 185
def panel(i, title):
    x = 12 + i * 196
    f.box("w", x, 14, W, 300)
    f.text(x + W / 2, 36, title, "h c")
    return x
# 1 queued message
x = panel(0, "A small queued message")
f.box("k", x + 14, 60, 157, 34, 3); f.text(x + 92, 74, "queue of the program's", "s c"); f.text(x + 92, 87, "channel", "s c")
for j, c in enumerate(["k", "k", "y"]):
    f.box("wk" if c == "k" else "y", x + 30 + j * 44, 104, 36, 22, 2)
f.text(x + 136, 119, "new", "s c")
f.arrow([(x + 48, 128), (x + 48, 160)], "lnk", "ak")
f.box("g", x + 14, 162, 157, 34, 3); f.text(x + 92, 176, "the thread takes it at", "s c"); f.text(x + 92, 189, "its next receive", "s c")
f.lines(x + 12, 222, ["Arrives in order with the", "program's other work.", "Carries a little data.", "Needs a receive loop."], "s", 14)
f.lines(x + 12, 290, ["suits: servers, event loops"], "s blue")
# 2 signal
x = panel(1, "A signal")
f.box("g", x + 14, 60, 157, 26, 3); f.text(x + 92, 77, "the thread is running", "s c")
f.arrow([(x + 92, 86), (x + 92, 108)], "lnr", "ar")
f.box("r", x + 14, 110, 157, 26, 3); f.text(x + 92, 127, "its handler runs instead", "s c")
f.arrow([(x + 92, 136), (x + 92, 158)], "ln", "a")
f.box("g", x + 14, 160, 157, 26, 3); f.text(x + 92, 177, "continues where it was", "s c")
f.lines(x + 12, 222, ["Needs no loop: it cuts in", "anywhere. The handler may", "do very little safely, and a", "blocked call fails early."], "s", 14)
f.lines(x + 12, 290, ["suits: simple programs"], "s blue")
# 3 wake
x = panel(2, "Waking a waiting thread")
f.box("y", x + 14, 60, 157, 34, 3); f.text(x + 92, 74, "a thread waits in a call", "s c"); f.text(x + 92, 87, "made for this event", "s c")
f.arrow([(x + 92, 94), (x + 92, 158)], "lng", "ag"); f.text(x + 100, 130, "the event", "s")
f.box("g", x + 14, 160, 157, 26, 3); f.text(x + 92, 177, "the call returns", "s c")
f.lines(x + 12, 222, ["The cheapest: nothing is", "queued or copied. One", "thread is tied to one", "source of events."], "s", 14)
f.lines(x + 12, 290, ["suits: interrupt threads"], "s blue")
# 4 new thread
x = panel(3, "Starting a new thread")
f.box("w", x + 14, 60, 157, 26, 3); f.text(x + 92, 77, "nobody is waiting", "s c")
f.arrow([(x + 92, 86), (x + 92, 108)], "lnk", "ak"); f.text(x + 100, 101, "the event", "s")
f.box("k", x + 14, 110, 157, 26, 3); f.text(x + 92, 127, "the system makes a thread", "s c")
f.arrow([(x + 92, 136), (x + 92, 158)], "lng", "ag")
f.box("g", x + 14, 160, 157, 26, 3); f.text(x + 92, 177, "it runs one function, ends", "s c")
f.lines(x + 12, 222, ["Nothing to prepare, and", "each event is handled on", "its own. The dearest: a", "stack and a thread each."], "s", 14)
f.lines(x + 12, 290, ["suits: rare events"], "s blue")
f.save("ch11-event-forms.svg")

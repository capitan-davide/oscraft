from svg import Fig
f = Fig(800, 300)
X0, W = 150, 60
def lane(y, name, sub):
    f.text(X0 - 14, y + 14, name, "b e"); f.text(X0 - 14, y + 28, sub, "s e"); f.line([(X0, y + 32), (770, y + 32)], "life")
def run(y, a, b, t, cls):
    f.box(cls, X0 + a * W, y, (b - a) * W, 30, 3); f.text(X0 + (a + b) * W / 2, y + 19, t, "s c")
lane(40, "H", "priority 30"); lane(90, "M", "priority 20"); lane(140, "L", "priority 10")
run(140, 0, 1, "runs", "k"); run(140, 1, 4, "holds the mutex: runs at 30", "y")
run(40, 4, 6.5, "locks, runs, unlocks", "g"); run(90, 6.5, 8.5, "runs", "u"); run(140, 8.5, 10, "runs at 10", "k")
for x, t, y in [(1, "L locks: raised to the ceiling", 196), (2, "M ready: cannot preempt 30", 212), (3, "H ready: waits its turn at 30", 228), (4, "L unlocks: back to 10", 196)]:
    pass
f.line([(X0 + 1 * W, 30), (X0 + 1 * W, 186)], "life"); f.text(X0 + 1 * W, 200, "L locks:", "s c"); f.text(X0 + 1 * W, 214, "raised at once", "s c")
f.arrow([(X0 + 2 * W, 76), (X0 + 2 * W, 88)], "ln", "a"); f.text(X0 + 2 * W, 70, "M ready", "s c")
f.arrow([(X0 + 3 * W, 26), (X0 + 3 * W, 38)], "ln", "a"); f.text(X0 + 3 * W, 20, "H ready", "s c")
f.line([(X0 + 4 * W, 30), (X0 + 4 * W, 186)], "life"); f.text(X0 + 4 * W, 200, "L unlocks:", "s c"); f.text(X0 + 4 * W, 214, "back to 10", "s c")
f.text(400, 256, "The mutex's ceiling is 30, the priority of the highest thread that uses it. From lock to unlock its holder runs at 30,", "s c")
f.text(400, 272, "so no thread that could want the mutex preempts it, and H never finds the mutex taken.", "s c")
f.save("ch09-ceiling.svg")

from svg import Fig
f = Fig(800, 250)
stages = [("n", "firmware", ["in ROM; runs at reset;", "brings up RAM and", "a boot device"]),
          ("n", "boot loader", ["finds the boot image,", "copies it to RAM,", "jumps into it"]),
          ("k", "early kernel code", ["learns what the machine", "is; builds page tables;", "sets up every CPU"]),
          ("k", "the kernel proper", ["scheduler, memory,", "interrupts, clock;", "creates the first process"]),
          ("u", "the first programs", ["start the servers and", "drivers; give the", "user a shell"])]
x, w, gap = 20, 136, 20
for i, (cls, t, ls) in enumerate(stages):
    f.box(cls, x, 60, w, 84)
    f.text(x + w / 2, 80, t, "b c")
    for j, l in enumerate(ls): f.text(x + w / 2, 100 + j * 14, l, "s c")
    if i < 4: f.arrow([(x + w, 102), (x + w + gap - 2, 102)], "ln", "a")
    x += w + gap
hand = ["the image, in RAM", "a CPU in a known state;", "a description to use", "one CPU running;", ""]
labels = [(20 + 136 + 10, ["RAM works"]), (20 + 2 * 156 - 10, ["the image in RAM;", "one CPU, in a", "known state"]),
          (20 + 3 * 156 - 10, ["a description of the", "machine; an address", "space; CPUs waiting"]), (20 + 4 * 156 - 10, ["a running kernel"])]
for x, ls in labels:
    f.line([(x, 104), (x, 166)], "life")
    for j, l in enumerate(ls): f.text(x, 180 + j * 13, l, "s c")
f.text(20, 30, "What each stage hands to the next", "h")
f.text(400, 236, "Each stage runs once and leaves the next one a little more to stand on. The earlier a stage, the more it is tied to one machine.", "s c")
f.save("ch05-boot-chain.svg")

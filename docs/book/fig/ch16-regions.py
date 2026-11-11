from svg import Fig
f = Fig(800, 440)
f.text(130, 24, "A process's memory map", "h c")
f.text(520, 24, "What stands behind each region", "h c")
rows = [
    ("shared memory object", "read, write; shared", "y", "y", "the object's frames", "allocated when the object was sized; another process maps the same ones"),
    ("device registers", "read, write; uncached", "n", "n", "the device, not RAM", "physical addresses outside the kernel's memory; never freed"),
    ("reservation", "no access", "w", "w", "nothing", "addresses kept, no frames until mprotect() makes them accessible"),
    ("heap, stack, anonymous memory", "read, write; private", "u", "u", "the process's own frames", "allocated and zeroed when mapped"),
    ("program data", "read, write; private", "u", "u", "the process's own frames", "allocated when mapped and filled from the file"),
    ("program code", "read, execute", "g", "g", "the boot image's own pages", "mapped in place, read-only; the same frames in every process running it"),
]
y = 40
for name, prot, c1, c2, back, note in rows:
    f.box(c1, 20, y, 220, 50, 3); f.text(130, y + 22, name, "c"); f.text(130, y + 38, prot, "s c")
    f.arrow([(240, y + 25), (300, y + 25)], "ln" if back != "nothing" else "life", "a")
    f.box(c2, 300, y, 480, 50, 3); f.text(312, y + 22, back, "b"); f.text(312, y + 38, note, "s")
    y += 60
f.arrow([(10, 392), (10, 44)]); f.text(22, 416, "addresses grow upwards; regions never overlap, and each records its permissions, its kind and how it was mapped", "s")
f.save("ch16-regions.svg")

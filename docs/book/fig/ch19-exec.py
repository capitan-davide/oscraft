from svg import Fig
f = Fig(800, 440)
f.text(400, 22, "execve() in process 1", "h c")
X, W = 180, 440
before = [("the caller may start programs (an ability)", ""),
          ("find the file; a script names its interpreter instead", ""),
          ("check the image without mapping it: headers, segments,", "the dynamic loader it names, enough free memory")]
after = [("end the process's other threads", ""),
         ("close the close-on-exec descriptors; caught signals back to default", ""),
         ("empty the address space", ""),
         ("map the new image, its stack and the system page; write the initial stack", ""),
         ("create a new first thread at the entry point; the calling thread ends", "")]
y = 40
n = 1
f.box("frame", X - 14, y - 6, W + 28, 150, 6)
for a, b in before:
    h = 40 if b else 30
    f.box("k", X, y, W, h, 3); f.num(X + 16, y + h/2, n)
    if b: f.text(X + 34, y + 17, a, "s"); f.text(X + 34, y + 31, b, "s")
    else: f.text(X + 34, y + h/2 + 4, a, "s")
    if n > 1: pass
    y += h + 10; n += 1
f.lines(X - 24, 80, ["A failure here is an", "ordinary error return:", "ENOENT, EACCES,", "ENOEXEC, ENOMEM ...", "The old program", "is untouched."], "s e", 14)
y += 12
f.line([(40, y), (760, y)], "lnr"); f.text(400, y - 6, "the point of no return", "b c red")
y += 22
y0 = y
f.box("frame", X - 14, y - 6, W + 28, 5 * 40 + 6, 6)
for a, b in after:
    h = 30
    f.box("u" if n < 7 else "g", X, y, W, h, 3); f.num(X + 16, y + h/2, n, "#9a6b2f" if n < 7 else "#3e7b34")
    f.text(X + 34, y + h/2 + 4, a, "s")
    y += h + 10; n += 1
f.lines(X + W + 24, y0 + 40, ["No program is left to", "return an error to.", "A failure here ends", "the process, as if", "killed by SIGKILL."], "s", 14)
f.lines(X - 24, y0 + 40, ["The process keeps its", "ID, its parent, its", "open descriptors and", "its ignored signals."], "s e", 14)
f.text(400, 428, "Everything that can fail for a reason the caller could act on is checked above the line.", "s c")
f.save("ch19-exec.svg")

from svg import Fig
f = Fig(800, 440)
S, K, N, R = 90, 300, 510, 710
for x, cls, t, sub in [(S, "u", "the sender", "has fd 5, an open file"), (K, "k", "the kernel", ""), (N, "k", "netd", "the AF_UNIX socket pair"), (R, "u", "the receiver", "")]:
    f.box(cls, x - 85, 14, 170, 42); f.text(x, 32, t, "b c"); f.text(x, 48, sub, "s c")
    f.line([(x, 56), (x, 430)], "life")
def step(n, y, x0, x1, text, cls="lnk", m="ak", col="#2c5d9b"):
    f.arrow([(x0, y), (x1, y)], cls, m)
    f.num(min(x0, x1) + 20, y - 13, n, col); f.text(min(x0, x1) + 36, y - 9, text, "s")
step(1, 92, S, K - 2, "register fd 5 for netd")
f.text(K + 8, 96, "returns a handle", "s")
f.arrow([(S, 138), (N - 2, 138)], "lnk", "ak"); f.num(S + 20, 125, 2, "#2c5d9b"); f.text(S + 36, 129, "sendmsg(): the data, and the handle in place of fd 5", "s")
step(3, 168, N, K + 2, "look the handle up")
f.box("wk", N - 85, 178, 170, 58, 3); f.text(N, 196, "connects to the file's server:", "s c"); f.text(N, 211, "netd now holds its own copy,", "s c"); f.text(N, 226, "queued with the data", "s c")
f.arrow([(R, 262), (N + 2, 262)], "lnk", "ak"); f.num(N + 22, 249, 4, "#2c5d9b"); f.text(N + 38, 253, "recvmsg()", "s")
step(5, 296, N, K + 2, "register the copy for the receiver")
step(6, 330, N, R - 2, "the data, and the new handle", "lng", "ag", "#3e7b34")
f.arrow([(R, 392), (K + 2, 392)], "lnk", "ak"); f.num(K + 22, 379, 7, "#2c5d9b"); f.text(K + 38, 383, "look it up; connect to the file's server", "s")
f.box("g", R - 85, 402, 170, 24, 3); f.text(R, 418, "a new descriptor, say fd 7", "s c")
f.box("wk", N - 85, 340, 170, 22, 3); f.text(N, 355, "closes its copy", "s c")
f.save("ch28-rights.svg")

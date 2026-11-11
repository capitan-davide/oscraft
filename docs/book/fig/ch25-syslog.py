from svg import Fig
f = Fig(800, 400)
# the program
f.box("u", 20, 20, 230, 250); f.text(135, 42, "a program", "b c")
f.box("g", 40, 56, 190, 44, 3); f.text(135, 75, "FreeBSD's libc", "b c"); f.text(135, 91, "syslog(): connect(), send()", "s c")
f.box("k", 40, 112, 190, 62, 3); f.text(135, 131, "libsys", "b c"); f.text(135, 147, "sees the path /var/run/log", "s c"); f.text(135, 162, "and takes the socket's place", "s c")
f.arrow([(135, 100), (135, 110)])
# the ring
f.box("y", 300, 150, 200, 120, 3); f.text(400, 171, "the process's log ring", "b c"); f.text(400, 188, "shared memory, mapped by both", "s c")
for i in range(6):
    f.box("g" if i in (1, 2, 3) else "w", 318 + i * 28, 204, 28, 26, 0)
f.text(400, 248, "/dev/shmem/logring/<pid>.<n>", "ms c")
# syslogd
f.box("u", 550, 20, 230, 250); f.text(665, 42, "syslogd", "b c")
f.box("wu", 570, 56, 190, 44, 3); f.text(665, 75, "/var/run/log", "m c"); f.text(665, 91, "a read registers the caller", "s c")
f.box("wu", 570, 160, 190, 44, 3); f.text(665, 179, "drain", "b c"); f.text(665, 195, "reads every ring until empty", "s c")
f.box("wu", 570, 214, 190, 44, 3); f.text(665, 233, "rules of /etc/syslog.conf", "b c"); f.text(665, 249, "facility and level select outputs", "s c")
f.arrow([(665, 204), (665, 212)])
# 1 register
f.arrow([(230, 124), (270, 124), (270, 70), (568, 70)], "lnk", "ak"); f.num(300, 56, 1, "#2c5d9b"); f.text(316, 60, "open and read", "s")
f.arrow([(568, 88), (290, 88), (290, 142), (232, 142)], "lng", "ag"); f.num(420, 102, 2, "#3e7b34"); f.text(436, 106, "the ring's name", "s")
# 3 write record
f.arrow([(230, 166), (262, 166), (262, 216), (298, 216)], "ln"); f.num(72, 202, 3); f.text(88, 200, "writes a record:", "s"); f.text(88, 214, "no kernel call", "s")
# 4 pulse
f.arrow([(52, 174), (52, 300), (665, 300), (665, 272)], "lnk", "ak", True); f.num(330, 300, 4, "#2c5d9b"); f.text(346, 318, "a pulse, only if syslogd said it was going to sleep", "s")
# 5 drain
f.arrow([(568, 182), (502, 182)], "ln", "a", False, False); f.num(535, 166, 5)
# outputs
f.box("y", 550, 340, 110, 40, 3); f.text(605, 357, "/var/log/", "ms c"); f.text(605, 371, "messages", "ms c")
f.box("y", 670, 340, 110, 40, 3); f.text(725, 364, "/dev/console", "ms c")
f.arrow([(620, 270), (605, 338)]); f.arrow([(710, 270), (725, 338)])
f.save("ch25-syslog.svg")

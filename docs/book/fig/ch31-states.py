from svg import Fig
f = Fig(800, 220)
f.box("g", 60, 60, 260, 100); f.text(190, 82, "raw", "h c"); f.lines(190, 102, ["/dev/md0 can be opened,", "read and written block by block", "(newfs_msdos writes a file system)"], "s c", 15)
f.box("k", 480, 60, 260, 100); f.text(610, 82, "mounted", "h c"); f.lines(610, 102, ["the blocks are the file system's;", "opening /dev/md0 fails with EBUSY;", "the mount point's files are served"], "s c", 15)
f.arrow([(320, 90), (478, 90)], "lnk", "ak"); f.text(400, 82, "mount: block 0 is FAT", "s c")
f.arrow([(478, 134), (322, 134)], "lng", "ag"); f.text(400, 152, "unmount: sync, then", "s c"); f.text(400, 166, "a fresh cache", "s c")
f.arrow([(140, 60), (140, 30), (240, 30), (240, 58)], "lnr", "ar"); f.text(190, 22, "mount, not FAT: EINVAL", "s c red")
f.arrow([(560, 160), (560, 190), (660, 190), (660, 162)], "lnr", "ar"); f.text(610, 208, "unmount with files open: EBUSY, unless forced", "s c red")
f.save("ch31-states.svg")

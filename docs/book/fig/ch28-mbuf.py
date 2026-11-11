from svg import Fig
f = Fig(800, 300)
f.text(400, 22, "one packet: a chain of mbufs", "h c")
# first mbuf
f.box("w", 30, 50, 370, 150); f.text(215, 68, "mbuf (the first of the packet)", "b c")
f.box("y", 44, 80, 130, 106, 3)
f.text(54, 98, "m_next", "m"); f.text(54, 116, "m_data", "m"); f.text(54, 134, "m_len = 54", "m"); f.text(54, 154, "the packet's length,", "s"); f.text(54, 168, "its interface", "s")
# its data area
f.box("w", 210, 100, 70, 40, 0); f.text(245, 124, "free", "s c")
f.box("g", 280, 100, 30, 40, 0); f.text(295, 124, "Eth", "s c")
f.box("k", 310, 100, 30, 40, 0); f.text(325, 124, "IP", "s c")
f.box("k", 340, 100, 40, 40, 0); f.text(360, 124, "TCP", "s c")
f.text(295, 92, "the mbuf's own storage", "s c")
f.arrow([(104, 112), (192, 112), (192, 154), (280, 154), (280, 142)], "ln", "a")
f.text(295, 174, "room in front for one", "s c"); f.text(295, 188, "more header", "s c")
# second mbuf
f.box("w", 450, 50, 150, 150); f.text(525, 68, "mbuf", "b c")
f.box("y", 464, 80, 122, 106, 3)
f.text(474, 98, "m_next = NULL", "m"); f.text(474, 116, "m_data", "m"); f.text(474, 134, "m_len = 1400", "m")
f.arrow([(104, 94), (192, 94), (192, 38), (525, 38), (525, 48)], "ln", "a")
# cluster
f.box("y", 650, 86, 130, 100, 3); f.text(715, 106, "a cluster:", "c"); f.text(715, 124, "a larger buffer,", "s c"); f.text(715, 138, "which chains", "s c"); f.text(715, 152, "can share", "s c"); f.text(715, 174, "the data", "b c")
f.arrow([(524, 112), (648, 112)], "ln", "a")
f.text(400, 236, "The layers do not copy a packet: they pass the pointer to its chain, add or remove", "s c")
f.text(400, 252, "headers at its front by moving m_data, and split or join chains by relinking mbufs.", "s c")
f.text(400, 276, "A socket's send and receive buffers are queues of such chains.", "s c")
f.save("ch28-mbuf.svg")

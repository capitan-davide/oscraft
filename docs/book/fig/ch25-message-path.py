from svg import Fig
f = Fig(800, 520)
def step(x, y, w, h, title, sub=None, cls="wk"):
    f.box(cls, x, y, w, h, 3)
    if sub: f.text(x + w / 2, y + h / 2 - 3, title, "b c"); f.text(x + w / 2, y + h / 2 + 13, sub, "s c")
    else: f.text(x + w / 2, y + h / 2 + 4, title, "b c")
# dispatch
f.text(20, 24, "dispatch", "h")
step(250, 12, 300, 40, "dispatch_block()", "MsgReceive into the thread's context")
f.arrow([(400, 52), (400, 72)])
step(250, 72, 300, 40, "dispatch_handler()", "the handler attached for this type or pulse code")
f.line([(10, 128), (790, 128)], "life")
# resmgr
f.text(20, 148, "resmgr", "h")
f.arrow([(400, 112), (400, 142)])
f.text(408, 124, "types 0x100 to 0x1ff", "s")
step(290, 142, 220, 34, "which kind of message?")
# connect branch
f.arrow([(290, 159), (160, 159), (160, 196)]); f.text(166, 152, "names a path", "s")
step(40, 196, 240, 44, "the attachment", "found by the handle in the message")
f.arrow([(160, 240), (160, 262)])
step(40, 262, 240, 44, "connect function", "open, unlink, rename, mknod ...", "wu")
f.arrow([(160, 306), (160, 328)])
step(40, 328, 240, 44, "bind", "this connection now means this OCB", "y")
# io branch
f.arrow([(510, 159), (640, 159), (640, 196)]); f.text(520, 152, "on an open file", "s")
step(520, 196, 240, 44, "the binding", "found by (scoid, pid, coid)")
f.arrow([(640, 240), (640, 262)])
step(520, 262, 240, 44, "I/O function, with the OCB", "read, write, stat, devctl, close ...", "wu")
# bind feeds bindings
f.arrow([(280, 350), (400, 350), (400, 218), (518, 218)], "ln", "a", True)
f.text(406, 296, "later", "s"); f.text(406, 310, "messages", "s")
# result
f.arrow([(160, 372), (160, 400), (330, 400)])
f.arrow([(640, 306), (640, 400), (472, 400)])
step(332, 384, 138, 34, "what it returned")
f.line([(10, 432), (790, 432)], "life")
f.text(20, 452, "reply", "h")
outs = [(90, "an error number", "MsgError: the call fails with it", "r"),
        (310, "0, or -n", "MsgReplyv: status and n parts", "g"),
        (530, "RESMGR_NOREPLY", "nothing: the function replied, or will", "y")]
for x, t, sub, cls in outs:
    f.box(cls, x, 458, 200, 48, 3); f.text(x + 100, 478, t, "m c"); f.text(x + 100, 495, sub, "s c")
f.arrow([(360, 418), (190, 456)]); f.arrow([(401, 418), (401, 456)]); f.arrow([(442, 418), (630, 456)])
f.save("ch25-message-path.svg")

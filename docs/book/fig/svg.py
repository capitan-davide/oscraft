"""Helpers for the book's figures: the style of docs/book/src/images and a
few shapes. A figure is a small script in this directory:

    from svg import Fig
    f = Fig(800, 300)
    f.box("k", 20, 20, 200, 40); f.text(120, 44, "the kernel", "c")
    f.arrow([(220, 40), (300, 40)])
    f.save("ch10-example.svg")        # written to ../src/images/

Run it from this directory (python3 ch10-example.py), then look at the
result: rsvg-convert -w 1000 ../src/images/ch10-example.svg -o /tmp/x.png.

Box classes: k (OsCraft, blue), u (a user process, tan), g (FreeBSD's code
or "running", green), y (waiting, a file, yellow), r (a fault, optional
QNX, red), n (hardware, grey), w/wk/wu (white insets). Text classes: c
(centred), e (right-aligned), b (bold), h (heading), s (small), m/ms
(monospace), red/blue/green. Line classes: ln, lnk (blue), lnr (red),
lng (green), life (a lifeline); arrows take the matching marker a, ak,
ar, ag."""
import os
HEAD = '''<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="DejaVu Sans, Helvetica, Arial, sans-serif">
  <defs><marker id="a" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#333333"/></marker><marker id="ak" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#2c5d9b"/></marker><marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#aa2222"/></marker><marker id="ag" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#3e7b34"/></marker></defs>
  <style>
    .k  {{ fill: #dce9f8; stroke: #2c5d9b; stroke-width: 1.2; }}
    .u  {{ fill: #f6e7d2; stroke: #9a6b2f; stroke-width: 1.2; }}
    .n  {{ fill: #eeeeee; stroke: #666666; stroke-width: 1.2; }}
    .g  {{ fill: #e2f0dc; stroke: #3e7b34; stroke-width: 1.2; }}
    .r  {{ fill: #f8e0e0; stroke: #aa2222; stroke-width: 1.2; }}
    .y  {{ fill: #fbf3c8; stroke: #9a8420; stroke-width: 1.2; }}
    .w  {{ fill: #ffffff; stroke: #666666; stroke-width: 1; }}
    .wk {{ fill: #ffffff; stroke: #2c5d9b; stroke-width: 1; }}
    .wu {{ fill: #ffffff; stroke: #9a6b2f; stroke-width: 1; }}
    .frame {{ fill: none; stroke: #999999; stroke-width: 1; stroke-dasharray: 5 4; }}
    .ln  {{ fill: none; stroke: #333333; stroke-width: 1.3; }}
    .lnk {{ fill: none; stroke: #2c5d9b; stroke-width: 1.3; }}
    .lnr {{ fill: none; stroke: #aa2222; stroke-width: 1.3; }}
    .lng {{ fill: none; stroke: #3e7b34; stroke-width: 1.3; }}
    .dash {{ stroke-dasharray: 5 4; }}
    .life {{ fill: none; stroke: #999999; stroke-width: 1; stroke-dasharray: 3 3; }}
    text {{ fill: #222222; font-size: 12px; }}
    .b  {{ font-weight: bold; }}
    .h  {{ font-size: 13px; font-weight: bold; }}
    .s  {{ font-size: 10.5px; fill: #444444; }}
    .m  {{ font-family: DejaVu Sans Mono, Menlo, Consolas, monospace; font-size: 11px; }}
    .ms {{ font-family: DejaVu Sans Mono, Menlo, Consolas, monospace; font-size: 10px; }}
    .c  {{ text-anchor: middle; }}
    .e  {{ text-anchor: end; }}
    .red {{ fill: #aa2222; }}
    .blue {{ fill: #2c5d9b; }}
    .green {{ fill: #3e7b34; }}
  </style>
  <rect x="0" y="0" width="{w}" height="{h}" fill="#ffffff"/>
'''
class Fig:
    def __init__(s, w, h): s.w, s.h, s.o = w, h, [HEAD.format(w=w, h=h)]
    def box(s, cls, x, y, w, h, rx=4): s.o.append(f'  <rect class="{cls}" x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}"/>\n')
    def text(s, x, y, t, cls="c"):
        t = t.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
        s.o.append(f'  <text x="{x}" y="{y}" class="{cls}">{t}</text>\n')
    def lines(s, x, y, ts, cls="c", dy=15):
        for i, t in enumerate(ts): s.text(x, y + i * dy, t, cls)
    def arrow(s, pts, cls="ln", m="a", dash=False, start=False):
        d = "M" + " L".join(f"{x},{y}" for x, y in pts)
        st = f' marker-start="url(#{m})"' if start else ""
        s.o.append(f'  <path class="{cls}{" dash" if dash else ""}" d="{d}" marker-end="url(#{m})"{st}/>\n')
    def line(s, pts, cls="ln", dash=False):
        d = "M" + " L".join(f"{x},{y}" for x, y in pts)
        s.o.append(f'  <path class="{cls}{" dash" if dash else ""}" d="{d}"/>\n')
    def num(s, x, y, n, fill="#333333"):
        s.o.append(f'  <circle cx="{x}" cy="{y}" r="9" fill="{fill}"/>\n  <text x="{x}" y="{y+4}" class="c" style="fill:#ffffff;font-size:11px;font-weight:bold">{n}</text>\n')
    def save(s, name):
        """Write the figure to docs/book/src/images/<name>."""
        here = os.path.dirname(os.path.abspath(__file__))
        open(os.path.join(here, "..", "src", "images", name), "w").write("".join(s.o) + "</svg>\n")

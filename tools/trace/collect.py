#!/usr/bin/env python3
"""Collect a kernel trace with QNX's tracelogger, in QEMU.

Usage: collect.py --qemu IMAGE [--run CMD]... [-o DIR]
                  [-- TRACELOGGER-OPTIONS]

Boots IMAGE in QEMU, starts
tracelogger with the options given (default: -n 4, four 16 KB buffers
per CPU's worth of events; -s SECS for a duration), runs each --run
command while it logs, waits for it, and copies the trace off over the
console (base64). The QNX SDP's traceprinter then decodes it: source the
SDP's qnxsdp-env.sh first (QNX_HOST).

IMAGE may be OsCraft's (build/oscraft.ifs) or the QNX reference
(build/qnx-reference.ifs, make qnx-image): the same commands traced on
both make a comparison (analyze.py --compare).

DIR (default build/trace/<time>) receives trace.kev, trace.txt
(traceprinter's text) and console.log. Then:

    python3 -I tools/trace/analyze.py DIR/trace.txt
"""

import argparse
import base64
import os
import re
import subprocess
import sys
import time

sys.dont_write_bytecode = True  # no __pycache__ in the tree
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from console import ROOT, Console  # noqa: E402

REMOTE = "/tmp/oscraft-trace.kev"


def copy(con, secs):
    """The trace file's bytes, over the console in base64; copied again
    if other output got mixed in (programs print on the console too)."""
    size = (con.command(f"wc -c < {REMOTE}", 10) or "").strip()
    if not size.isdigit():
        sys.exit(f"collect.py: no trace file ({size!r}): tracelogger failed?")
    for _ in range(3):
        # The markers are split in the command so that its echo doesn't match.
        text = con.command(f'echo "==TRACE""-BEGIN=="; base64 {REMOTE}; echo "==TRACE""-END=="', secs)
        m = re.search(r"==TRACE-BEGIN==\n(.*?)==TRACE-END==", text or "", re.S)
        try:
            data = base64.b64decode("".join(m.group(1).split()), validate=True) if m else b""
        except ValueError:
            data = b""
        if len(data) == int(size):
            con.command(f"rm {REMOTE}", 10)
            return data
        print("collect.py: the copy was garbled, again", file=sys.stderr, flush=True)
        con.settle(2, 30)
    sys.exit("collect.py: could not copy the trace intact")


def main():
    argv = sys.argv[1:]
    opts = []
    if "--" in argv:
        i = argv.index("--")
        argv, opts = argv[:i], argv[i + 1:]
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--qemu", metavar="IMAGE", required=True, help="boot IMAGE in QEMU")
    p.add_argument("--run", metavar="CMD", action="append", default=[], help="workload command")
    p.add_argument("-o", dest="out", help="output directory")
    p.add_argument("--timeout", type=float, default=120, help="seconds for tracelogger and each command")
    a = p.parse_args(argv)
    opts = opts or ["-n", "4"]
    if not os.environ.get("QNX_HOST"):
        sys.exit("collect.py: QNX_HOST not set: source your QNX SDP's qnxsdp-env.sh")

    out = a.out or os.path.join(ROOT, "build/trace", time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(out, exist_ok=True)
    con = Console.qemu(a.qemu, os.path.join(out, "console.log"))
    try:
        if not con.wait_prompt(180):
            sys.exit("collect.py: no shell prompt (has the system booted?)")
        con.settle(3, 60)
        logger = f"tracelogger {' '.join(opts)} -f {REMOTE}"
        if a.run:
            con.command(f"{logger} >/dev/null 2>&1 &", 10)
            time.sleep(1)  # the logger attaches and starts
            for cmd in a.run:
                print(f"running: {cmd}", flush=True)
                if con.command(cmd, a.timeout) is None:
                    print(f"collect.py: no prompt after {cmd!r} within {a.timeout:.0f}s", file=sys.stderr)
            if con.command("wait", a.timeout) is None:
                sys.exit("collect.py: tracelogger didn't finish (more buffers than the workload fills? "
                         "give -s SECS)")
        elif con.command(logger, a.timeout) is None:
            sys.exit("collect.py: tracelogger didn't finish")
        data = copy(con, a.timeout * 4)
    finally:
        con.close()
    kev = os.path.join(out, "trace.kev")
    open(kev, "wb").write(data)
    txt = os.path.join(out, "trace.txt")
    with open(txt, "w") as f:
        subprocess.run([os.path.join(os.environ["QNX_HOST"], "usr/bin/traceprinter"), "-f", kev],
                       stdout=f, check=True)
    events = sum(1 for line in open(txt) if line.startswith("t:"))
    print(f"{kev}: {os.path.getsize(kev)} bytes, {events} events, decoded in {txt}")
    print(f"next: python3 -I tools/trace/analyze.py {os.path.relpath(txt, ROOT)}")


if __name__ == "__main__":
    main()

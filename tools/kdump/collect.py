#!/usr/bin/env python3
"""Collect a kernel dump from OsCraft in QEMU.

Usage: collect.py --qemu IMAGE [--run CMD] [--kernel ELF]
                  [-o DIR] [--timeout SECS]

The image must have QNX's kdumper (make image KDUMP=1, which needs the
QNX SDP: kdumper is QNX's). With
--run, CMD is typed at the shell once it is up (e.g. kcrash, or the
program that crashes the kernel); without it, the crash is waited for
(during boot, or caused some other way). The console is recorded in one
session from start to the dump's end: a second reader in between loses
characters and damages the dump.

DIR (default build/kdump/<time>) receives console.log, kdump.elf and a
copy of the kernel ELF (--kernel, default the release kernel), which
must be the one in the image for the dump to make sense. Then:

    python3 -I tools/kdump/analyze.py DIR

QEMU writes a dump in seconds (compressed, kdumper -C).
"""

import argparse
import os
import shutil
import sys
import time

sys.dont_write_bytecode = True  # no __pycache__ in the tree
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
from console import ROOT, Console  # noqa: E402
from kdump.extract import extract  # noqa: E402

KERNEL = os.path.join(ROOT, "target/aarch64-unknown-none-softfloat/release/kernel")


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--qemu", metavar="IMAGE", required=True, help="boot IMAGE in QEMU")
    p.add_argument("--run", metavar="CMD", help="shell command that crashes the kernel")
    p.add_argument("--kernel", default=KERNEL, help="the image's kernel ELF")
    p.add_argument("-o", dest="out", help="output directory")
    p.add_argument("--timeout", type=float, default=600, help="seconds to wait for the dump")
    a = p.parse_args()

    out = a.out or os.path.join(ROOT, "build/kdump", time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(out, exist_ok=True)
    log = os.path.join(out, "console.log")
    if os.path.exists(a.kernel):
        shutil.copy(a.kernel, os.path.join(out, "kernel"))
    else:
        print(f"collect.py: no kernel ELF at {a.kernel}; analysis needs it", file=sys.stderr)

    con = Console.qemu(a.qemu, log)
    try:
        start = con.mark()
        if a.run:
            if not con.wait_prompt(180):
                sys.exit("collect.py: no shell prompt (has the system booted?)")
            print(f"running: {a.run}", flush=True)
            con.send(a.run + "\r")
        if not con.wait_for(start, b"begin 644 kdump", a.timeout):
            sys.exit(f"collect.py: no dump within {a.timeout:.0f}s (is kdumper in the image?); console in {log}")
        print("dump started", flush=True)
        if not con.wait_for(start, b"`\r\nend\r\n", a.timeout) and not con.wait_for(start, b"`\nend\n", 1):
            sys.exit(f"collect.py: the dump did not end within {a.timeout:.0f}s; console in {log}")
        time.sleep(0.5)
    finally:
        con.close()
    try:
        print(extract(log, os.path.join(out, "kdump.elf")))
    except ValueError as e:
        sys.exit(f"collect.py: {e}; console in {log}")
    print(f"next: python3 -I tools/kdump/analyze.py {os.path.relpath(out, ROOT)}")


if __name__ == "__main__":
    main()

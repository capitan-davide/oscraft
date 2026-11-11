#!/usr/bin/env python3
"""Analyze a kernel dump written by QNX's kdumper (collect.py's output).

Usage: analyze.py DIR | DUMP [--kernel ELF] [--console LOG] [-x GDBCMD]...

DIR is a collect.py directory (kdump.elf, kernel, console.log). Prints:
- the crash from the dump's note: signal, fault, CPU, pc, sp;
- whether the kernel ELF is the one that crashed (its code is in the
  dump);
- the kernel's own report from the console (the exception and its
  registers, the thread and its kernel stack, or the panic message);
- gdb's view through QNX's kdserver: backtrace with locals, registers,
  the instructions at the pc, and any -x commands (e.g. -x 'frame 2'
  -x 'info locals', -x 'p *some_static').

gdb and kdserver come from the QNX SDP: source its qnxsdp-env.sh first
(QNX_HOST). kdserver reads kernel addresses through
the dump's translation tables: kernel statics, heap and stacks are
readable, user memory is not (not in a default dump).
"""

import argparse
import os
import re
import struct
import subprocess
import sys

SIGNALS = {4: "SIGILL", 5: "SIGTRAP", 6: "SIGABRT", 7: "SIGBUS", 8: "SIGFPE", 10: "SIGBUS",
           11: "SIGSEGV"}
FAULTS = {1: "FLTILL", 2: "FLTPRIV", 3: "FLTBPT", 4: "FLTTRACE", 5: "FLTACCESS",
          6: "FLTBOUNDS", 7: "FLTIOVF", 8: "FLTIZDIV", 9: "FLTFPE", 10: "FLTSTACK", 11: "FLTPAGE"}


def phdrs(d):
    """(type, offset, paddr, filesz) of each program header, ELF32 or ELF64."""
    if d[:4] != b"\x7fELF":
        raise ValueError("not an ELF file")
    if d[4] == 1:
        phoff, phentsize, phnum = struct.unpack_from("<I", d, 28)[0], *struct.unpack_from("<HH", d, 42)
        return [(lambda t, o, v, p, fs, *_: (t, o, p, fs))(*struct.unpack_from("<8I", d, phoff + i * phentsize))
                for i in range(phnum)]
    phoff, phentsize, phnum = struct.unpack_from("<Q", d, 32)[0], *struct.unpack_from("<HH", d, 54)
    out = []
    for i in range(phnum):
        t, _, o, _, p, fs = struct.unpack_from("<IIQQQQ", d, phoff + i * phentsize)
        out.append((t, o, p, fs))
    return out


# The note's layout, from <sys/kdump.h> and <aarch64/context.h> in
# $QNX_TARGET/usr/include: struct kdump_note (40 bytes: four bytes,
# two 16-bit fields, eight 32-bit ones, the last extra_reg_name_size),
# then the extra registers' names, then one struct kdump_note_cpu per CPU
# (pgtbl, cpunum, spare[4], process_name[100]: 128 bytes), each followed
# by its AARCH64_CPU_REGISTERS (gpr[32], elr, pstate: 34 64-bit words).
KDUMP_NOTE = "<4B2H8I"
KDUMP_NOTE_SIZE = struct.calcsize(KDUMP_NOTE)
KDUMP_NOTE_CPU_SIZE = 8 + 4 + 4 * 4 + 100
CPU_REGISTERS = "<34Q"


def note(d, ph):
    """The kdump note: signal, code, fault, CPU, registers (of the first
    kdump_note_cpu, the faulting CPU's)."""
    t, off, _, size = next(p for p in ph if p[0] == 4)
    n = d[off:off + size]
    (_version, signo, code, fault, _ncpus, cpu, _regset_size, _cpu_info, _procnto_seg, _syspage_seg,
     _dumpinfo_seg, _cpu_info_hi, _extra_value_size, extra_name_size) = struct.unpack_from(KDUMP_NOTE, n, 0)
    x = struct.unpack_from(CPU_REGISTERS, n, KDUMP_NOTE_SIZE + extra_name_size + KDUMP_NOTE_CPU_SIZE)
    return {"signo": signo, "code": code, "fault": fault, "cpu": cpu,
            "x": x[:31], "sp": x[31], "pc": x[32], "pstate": x[33]}


def kernel_matches(d, kernel):
    """Whether the kernel ELF's code is in the dump: pieces from across
    its code segment, all found (a different build fails some)."""
    k = open(kernel, "rb").read()
    off, size = next((o, fs) for t, o, _, fs in phdrs(k) if t == 1)
    pieces = [off + i * (size - 4096) // 7 for i in range(8)]
    return all(d.find(k[o:o + 4096]) >= 0 for o in pieces)


def report(console):
    """The kernel's crash report: the lines before kdumper's output."""
    lines = open(console, "rb").read().decode(errors="replace").replace("\r", "").split("\n")
    end = next((i for i in range(len(lines) - 1, -1, -1) if lines[i].startswith("kernel crash:")), None)
    if end is None:
        end = next((i for i, l in enumerate(lines) if l.startswith("begin 644 kdump")), len(lines))
    start = end
    pat = re.compile(r"fault at level|exception from|kernel panic|crashing the kernel")
    for i in range(end - 1, max(end - 60, 0) - 1, -1):
        if pat.search(lines[i]):
            start = i
    return lines[start:end + 1]


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("dump", help="collect.py directory or kdump.elf")
    p.add_argument("--kernel", help="kernel ELF (default: DIR/kernel)")
    p.add_argument("--console", help="console log (default: DIR/console.log)")
    p.add_argument("-x", dest="cmds", action="append", default=[], help="extra gdb command")
    a = p.parse_args()

    base = a.dump if os.path.isdir(a.dump) else os.path.dirname(a.dump)
    dump = os.path.join(a.dump, "kdump.elf") if os.path.isdir(a.dump) else a.dump
    kernel = a.kernel or os.path.join(base, "kernel")
    console = a.console or os.path.join(base, "console.log")

    d = open(dump, "rb").read()
    ph = phdrs(d)
    nt = note(d, ph)
    print(f"== crash: {SIGNALS.get(nt['signo'], nt['signo'])} code {nt['code']} "
          f"{FAULTS.get(nt['fault'], nt['fault'])} on CPU {nt['cpu']}")
    print(f"   pc {nt['pc']:#x}  sp {nt['sp']:#x}  lr {nt['x'][30]:#x}  pstate {nt['pstate']:#x}")
    print(f"   {len(ph)} segments, {len(d) >> 20} MiB")
    if not os.path.exists(kernel):
        sys.exit(f"analyze.py: no kernel ELF ({kernel}); give --kernel")
    if not kernel_matches(d, kernel):
        print(f"== WARNING: {kernel} is not the kernel that crashed (its code differs from the dump's)")
    if os.path.exists(console):
        print("== the kernel's report (console):")
        for line in report(console):
            print("   " + line)

    host = os.environ.get("QNX_HOST")
    if not host:
        sys.exit("analyze.py: QNX_HOST not set: source your QNX SDP's qnxsdp-env.sh")
    bin_dir = os.path.join(host, "usr/bin")
    cmds = ["set pagination off", "set print pretty off", f"target remote | kdserver {dump}",
            "bt full 25", "info registers", "x/8i $pc - 16"] + a.cmds
    gdb = [os.path.join(bin_dir, "ntoaarch64-gdb"), "-nx", "-batch", "-q"]
    gdb += [arg for c in cmds for arg in ("-ex", c)] + [kernel]
    print("== gdb (kdserver):", flush=True)
    # gdb starts kdserver by name, from the SDP's tools.
    env = dict(os.environ, PATH=bin_dir + os.pathsep + os.environ.get("PATH", ""))
    r = subprocess.run(gdb, env=env, capture_output=True, text=True)
    for line in (r.stdout + r.stderr).splitlines():
        # Missing Rust std sources: noise.
        if "No such file or directory" in line or line.startswith("warning: ") and "/rustc/" in line:
            continue
        print("   " + line)


if __name__ == "__main__":
    main()

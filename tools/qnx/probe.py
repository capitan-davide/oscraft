#!/usr/bin/env python3
"""Run one C program on QNX's procnto and on OsCraft, in QEMU, and print
both outputs: the way OsCraft's behaviour is measured against procnto's.

The program is built with the SDP's GCC against QNX's libc, added to the
QNX reference image (tests/qnx/reference.build) and to OsCraft's test image
(images/qemu-virt.build with the QNX overlay and the tests' files) as /proc/boot/probe,
and run at each one's shell.
The images are written to build/probe/; the normal images are untouched.

Usage: probe.py [options] prog.c [args...]
  --oscraft-only    skip the QNX run
  --qnx-only        skip the OsCraft run
  --procnto-opts S  extra options for the kernel in both images (e.g. "-C 1")
  --cflags S        extra compiler options (e.g. "-lm")
  --timeout SECS    how long the program may run (default 30)
  --profile P       OsCraft kernel profile, dev (default) or release
  --diff            print a unified diff of the two outputs as well

It needs the QNX SDP's environment (QNX_HOST, QNX_TARGET: source the
SDP's qnxsdp-env.sh). Both images boot with OsCraft's startup for QEMU's
virt (make startup).

QEMU runs one at a time: don't start a probe while `make test` runs.
"""

import argparse
import difflib
import importlib.util
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "build/probe")
TESTS = os.path.join(ROOT, "build/qnx-tests")
SSH_KEY = os.path.join(ROOT, "build/ssh_host_ed25519_key")
KERNEL_TARGET = "aarch64-unknown-none-softfloat"

# The test runner's QEMU target (tools/test/run-tests.py), imported
# without leaving bytecode in the tree.
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location("run_tests", os.path.join(ROOT, "tools/test/run-tests.py"))
run_tests = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(run_tests)


def sh(args, **kw):
    subprocess.run(args, cwd=ROOT, check=True, **kw)


def check_env():
    """The SDP is the user's, installed anywhere."""
    missing = [v for v in ("QNX_HOST", "QNX_TARGET") if not os.environ.get(v)]
    if missing:
        sys.exit(f"probe.py: {', '.join(missing)} not set: source your QNX SDP's qnxsdp-env.sh")


def build_program(src, cflags):
    exe = os.path.join(OUT, "probe")
    sh([os.path.join(os.environ["QNX_HOST"], "usr/bin/ntoaarch64-gcc"), "-O2", "-o", exe, os.path.abspath(src)]
       + cflags.split())
    return exe


def missing_tests():
    """Whether a test program either image lists hasn't been built (a
    test added since the last `make qnx-tests`: mkifs would fail on it)."""
    for build in ("tests/qnx/image.build", "tests/qnx/reference.build"):
        for line in open(os.path.join(ROOT, build)):
            _, sep, rest = line.partition("${OSCRAFT_QNX_TESTS}/")
            if sep and not os.path.exists(os.path.join(TESTS, rest.split()[0])):
                return True
    return False


def write_build(template, out, edits, exe):
    """The image's build file with `edits` (old, new) applied to its lines
    and the program added."""
    lines = []
    for line in open(os.path.join(ROOT, template)):
        for old, new in edits:
            if line is not None and old(line):
                line = new(line)
        if line is not None:
            lines.append(line)
    lines.append(f"\n/proc/boot/probe={exe}\n")
    with open(out, "w") as f:
        f.writelines(lines)


def mkifs(build, image, env):
    env = dict(os.environ, OSCRAFT_QNX_TESTS=TESTS, **env)
    sh([os.path.join(ROOT, "target/release/mkifs"), "-o", os.path.join(OUT, "sym"),
        "-r", os.path.join(ROOT, "build/startup-root"), build, image], env=env,
       stdout=subprocess.DEVNULL)


def qnx_image(exe, opts):
    build, image = os.path.join(OUT, "qnx.build"), os.path.join(OUT, "qnx.ifs")
    edits = [(lambda l: "procnto-smp-instr" in l and opts, lambda l: l.rstrip("\n") + f" {opts}\n")]
    write_build("tests/qnx/reference.build", build, edits, exe)
    mkifs(build, image, {})
    return image


def oscraft_image(exe, opts, profile):
    build, image = os.path.join(OUT, "oscraft.build"), os.path.join(OUT, "oscraft.ifs")
    # The build file `make test QNX=1` uses: written by the Makefile, so
    # that the probe runs the system that measures.
    sh(["make", "-s", "QNX=1", "test-build"])
    edits = [(lambda l: "${OSCRAFT_KERNEL}" in l and opts, lambda l: l.rstrip("\n") + f" {opts}\n")]
    write_build("build/oscraft-test.build", build, edits, exe)
    pdir = "debug" if profile == "dev" else profile
    tdir = os.path.join(ROOT, "target", KERNEL_TARGET, pdir)
    # The build file's variables, the Makefile's; the kernel's as profiled.
    out = subprocess.run(["make", "-s", "image-env"], cwd=ROOT, check=True, capture_output=True, text=True).stdout
    env = dict(v.split("=", 1) for v in out.split())
    env.update({"OSCRAFT_KERNEL": os.path.join(tdir, "kernel"), "OSCRAFT_USER": tdir, "OSCRAFT_SSH_KEY": SSH_KEY})
    env.pop("OSCRAFT_QNX_TESTS", None)
    mkifs(build, image, env)
    return image


def run(image, ready, cmd, secs, boot=15):
    target = run_tests.Target(image)
    try:
        if not target.wait_for(0, ready, 90):
            return f"(no '{ready.decode()}' on the console)\n" + target.since(0).decode(errors="replace")[-2000:]
        at = target.since(0).index(ready)
        if not target.wait_for(at, run_tests.PROMPT, boot):
            return "(no shell prompt)\n" + target.since(at).decode(errors="replace")[-1500:]
        return target.command(cmd, secs)
    finally:
        target.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--oscraft-only", action="store_true")
    ap.add_argument("--qnx-only", action="store_true")
    ap.add_argument("--procnto-opts", default="")
    ap.add_argument("--cflags", default="")
    ap.add_argument("--timeout", type=float, default=30)
    ap.add_argument("--profile", default="dev")
    ap.add_argument("--diff", action="store_true")
    ap.add_argument("prog")
    ap.add_argument("args", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    check_env()

    os.makedirs(os.path.join(OUT, "sym"), exist_ok=True)
    targets = ["mkifs", "startup"]
    if not a.qnx_only:
        targets += ["kernel", "user", os.path.relpath(SSH_KEY, ROOT)]
    sh(["make", "-s", f"PROFILE={a.profile}"] + targets)
    if missing_tests():
        sh(["make", "-s", "qnx-tests"])
    exe = build_program(a.prog, a.cflags)
    cmd = " ".join(["probe"] + a.args)

    outs = {}
    if not a.oscraft_only:
        outs["QNX procnto"] = run(qnx_image(exe, a.procnto_opts), b"Welcome to QNX", cmd, a.timeout)
    if not a.qnx_only:
        outs["OsCraft"] = run(oscraft_image(exe, a.procnto_opts, a.profile), b"bootscript: done", cmd,
                              a.timeout)
    for name, out in outs.items():
        print(f"===== {name}")
        print(out)
    if a.diff and len(outs) == 2:
        q, o = outs.values()
        sys.stdout.writelines(difflib.unified_diff((q + "\n").splitlines(True), (o + "\n").splitlines(True), "procnto", "oscraft"))
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Boot an OsCraft image in QEMU and run the target tests against it.

The console (the shell) is a QEMU TCP serial port. Each test sends shell
commands and checks their output. Prints one line per test and exits
non-zero if any failed. The whole console is saved to
build/test-console.log.

Usage: run-tests.py [--qnx | --procnto] [--stop] <image> <tests file> [test name...]

Without options, the native tests run (`make test`: the base image, FreeBSD's
sh on the kernel's console). --qnx runs the QNX personality's tests too,
marked `needs: qnx` (`make test QNX=1`: the image with the QNX overlay,
QNX's ksh on its serial driver as the console). --procnto boots QNX's own
kernel's image instead (`make qnx-test`, build/qnx-reference.ifs) and
runs the QNX tests there: it waits for that image's welcome line rather
than OsCraft's "bootscript: done". The QNX tests' expected values are
procnto's measured results, so they pass on both.

--stop ends the run at the first failure, but for ipi, whose timing is the
host's: a failure is fixed before the rest are worth running.

Tests file: blocks separated by blank lines, `#` comments.
    name: spawn                    the test's name
    run: spawntest                 a shell command (several allowed)
    expect: spawntest: passed      a regex the output must match (several)
    reject: FAIL                   a regex the output must not match
    timeout: 10                    seconds per command (default 8)
    gdb: tests/gdb/dbgtest.gdb     a gdb script, instead of shell commands
    pre: dbgtest spin &            shell commands before the gdb script
    needs: qnx                     a test of the QNX personality: runs
                                   only with --qnx or --procnto
A gdb test runs the SDP's gdb against pdebug on a second serial line, which
QEMU's virt machine has not: they are skipped there, with a note.
The file is checked before anything boots: an unknown key, a test with
neither expect nor reject, run and gdb together, pre without gdb, a bad
regex or a repeated name stops the run.

A command still running after its timeout fails its test ("timed out");
the harness then sends ^C and waits for the shell to echo a marker, so the
late output isn't taken for the next test's.
"""

import os
import re
import signal
import socket
import subprocess
import sys
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# The shell's prompt as it boots. A test run then sets one no program prints
# at the start of a line (`Target.use_test_prompt`): "# " alone also matched
# any output line that began so, and ended a command early (TOOLS-10).
PROMPT = b"\n# "
TEST_PROMPT = b"@@# "
# After a command times out: how long its program may take to stop (it gets
# ^C) before the shell answers the resynchronisation marker.
RESYNC_SECS = 120


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


LISTS = ("run", "expect", "reject", "pre")
SINGLE = ("name", "timeout", "gdb", "needs")


def parse(path):
    """The tests in `path`, checked: a misspelt key, a test that asserts
    nothing or a bad regex is an error, not a test that passes vacuously
    (review TOOLS-9). Raises ValueError with every problem found."""
    tests, cur, errors = [], None, []
    for n, raw in enumerate(open(path), 1):
        line = raw.rstrip("\n")
        if not line.strip():
            cur = None
            continue
        if line.lstrip().startswith("#"):
            continue
        key, colon, value = line.partition(":")
        key, value = key.strip(), value.strip()
        if cur is None:
            cur = {"run": [], "expect": [], "reject": [], "pre": [], "timeout": 8.0, "line": n}
            tests.append(cur)
        where = f"{path}:{n}"
        if not colon or key not in LISTS + SINGLE:
            errors.append(f"{where}: unknown key {key!r} (known: {', '.join(LISTS + SINGLE)})")
        elif key in LISTS:
            cur[key].append(value)
        elif (key == "timeout" and "timeout_set" in cur) or (key != "timeout" and key in cur):
            errors.append(f"{where}: {key!r} given twice in one test")
        elif key == "timeout":
            try:
                cur[key] = float(value)
                cur["timeout_set"] = True
            except ValueError:
                errors.append(f"{where}: timeout {value!r} is not a number")
        else:
            cur[key] = value
    names = set()
    for t in tests:
        where = f"{path}:{t['line']}"
        name = t.get("name")
        if not name:
            errors.append(f"{where}: a test without a name")
        elif name in names:
            errors.append(f"{where}: test {name!r} defined twice")
        names.add(name)
        if not t["expect"] and not t["reject"]:
            errors.append(f"{where}: test {name!r} has no expect or reject: it would assert nothing")
        if ("gdb" in t) == bool(t["run"]):
            errors.append(f"{where}: test {name!r} needs either run lines or a gdb script, not "
                          + ("both" if t["run"] else "neither"))
        if t["pre"] and "gdb" not in t:
            errors.append(f"{where}: test {name!r} has pre lines, which only a gdb test runs")
        if t.get("needs", "qnx") != "qnx":
            errors.append(f"{where}: test {name!r}: needs must be qnx")
        for pattern in t["expect"] + t["reject"]:
            try:
                re.compile(pattern)
            except re.error as e:
                errors.append(f"{where}: test {name!r}: bad regex {pattern!r}: {e}")
    if errors:
        raise ValueError("\n".join(errors))
    return tests


class Target:
    """QEMU with its console on a TCP socket, read in the background."""

    # pdebug's line, for the gdb tests: none on QEMU's virt machine.
    dbg_port = None

    def __init__(self, image):
        self.con_port = free_port()
        env = dict(os.environ, SERIAL1=f"tcp:127.0.0.1:{self.con_port},server,wait")
        # QEMU's own messages go to a file, so that a QEMU that fails to
        # start says why (review TOOLS-18: they went to /dev/null).
        os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
        qemu_log = os.path.join(ROOT, "build", "qemu-output.log")
        with open(qemu_log, "wb") as out:
            self.qemu = subprocess.Popen([os.path.join(ROOT, "tools/qemu/run.sh"), image], env=env,
                                         stdout=out, stderr=subprocess.STDOUT,
                                         start_new_session=True)
        for _ in range(100):
            try:
                self.sock = socket.create_connection(("127.0.0.1", self.con_port))
                break
            except OSError:
                time.sleep(0.1)
        else:
            with open(qemu_log, errors="replace") as f:
                said = f.read()[-2000:].strip()
            raise RuntimeError("QEMU's console didn't come up" + (f"; QEMU said:\n{said}" if said else ""))
        self.sock.settimeout(0.2)
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.stop = False
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        while not self.stop:
            try:
                data = self.sock.recv(4096)
            except socket.timeout:
                continue
            except OSError:
                return
            if not data:
                return
            with self.lock:
                self.buf += data

    def mark(self):
        with self.lock:
            return len(self.buf)

    def since(self, mark):
        with self.lock:
            return bytes(self.buf[mark:])

    def wait_for(self, mark, pattern, secs):
        end = time.time() + secs
        while time.time() < end:
            if pattern in self.since(mark):
                return True
            time.sleep(0.05)
        return False

    prompt = PROMPT
    timed_out = False
    syncs = 0

    def send(self, data):
        self.sock.sendall(data)

    def use_test_prompt(self, secs=10):
        """Give the shell `TEST_PROMPT`; False if it didn't take it (the
        commands then go on with the boot prompt)."""
        mark = self.mark()
        self.send(b"PS1='" + TEST_PROMPT + b"'\r")
        if not self.wait_for(mark, b"\n" + TEST_PROMPT, secs):
            return False
        self.prompt = b"\n" + TEST_PROMPT
        return True

    def command(self, cmd, secs):
        """Run a shell command; its output up to the next prompt. A command
        still running after `secs` is reported as timed out (`timed_out`,
        and a line at the end of its output), then the shell is brought
        back in step (`resync`), so that its late output isn't read as the
        next command's (review TOOLS-10)."""
        mark = self.mark()
        self.send(cmd.encode() + b"\r")
        done = self.wait_for(mark, self.prompt, secs)
        out = self.since(mark).decode(errors="replace").replace("\r", "")
        # Drop the echoed command and the prompt.
        out = out.split("\n", 1)[1] if "\n" in out else ""
        self.timed_out = not done
        if done:
            return out.rsplit(self.prompt.decode(), 1)[0]
        out += f"\n(timed out after {secs:g} s)"
        if not self.resync():
            out += f"\n(and the shell didn't answer within {RESYNC_SECS} s of ^C)"
        return out

    def resync(self):
        """After a timeout: interrupt the command (^C), then have the shell
        echo a marker no earlier output can contain and wait for it and the
        prompt after it. What came before is the timed-out command's."""
        self.syncs += 1
        marker = f"@@sync{self.syncs}@@".encode()
        mark = self.mark()
        self.send(b"\x03")
        time.sleep(0.5)
        # The echo of the typed command starts with "echo ", so only the
        # marker's own output line begins with it.
        self.send(b"echo " + marker + b"\r")
        if not self.wait_for(mark, b"\n" + marker, RESYNC_SECS):
            return False
        after = mark + self.since(mark).index(b"\n" + marker)
        return self.wait_for(after, self.prompt, 10)

    def close(self):
        self.stop = True
        try:
            os.killpg(self.qemu.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.qemu.wait()


def run_gdb(target, script, secs):
    # The SDP's environment (source its qnxsdp-env.sh), as for the build.
    host, tgt = os.environ.get("QNX_HOST"), os.environ.get("QNX_TARGET")
    if not host or not tgt:
        return "QNX_HOST or QNX_TARGET not set: source your QNX SDP's qnxsdp-env.sh"
    args = [os.path.join(host, "usr/bin/ntoaarch64-gdb"), "-nx", "-batch", "-q",
            "-ex", "set pagination off", "-ex", "set confirm off",
            "-ex", f"set sysroot {tgt}/aarch64le",
            "-ex", f"set solib-search-path {tgt}/aarch64le/lib:{tgt}/aarch64le/usr/lib",
            "-ex", f"target qnx 127.0.0.1:{target.dbg_port}",
            "-x", os.path.join(ROOT, script)]
    try:
        r = subprocess.run(args, cwd=ROOT, capture_output=True, timeout=secs)
        return (r.stdout + r.stderr).decode(errors="replace")
    except subprocess.TimeoutExpired as e:
        return ((e.stdout or b"") + (e.stderr or b"")).decode(errors="replace") + "\n(gdb timed out)"


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        return 2
    args = sys.argv[1:]
    stop = "--stop" in args
    stopped = False
    passed = 1  # the boot
    args = [a for a in args if a != "--stop"]
    qnx = args[:1] == ["--qnx"]
    procnto = args[:1] == ["--procnto"]
    if qnx or procnto:
        args = args[1:]
    if len(args) < 2:
        print(__doc__)
        return 2
    image = args[0]
    try:
        tests = parse(args[1])
    except ValueError as e:
        print(f"malformed tests file:\n{e}")
        return 2
    only = set(args[2:])
    if only:
        tests = [t for t in tests if t.get("name") in only]
    target = Target(image)
    start = 0
    failed = []
    skipped = 0
    try:
        done = b"Welcome to QNX" if procnto else b"bootscript: done"
        if not target.wait_for(start, done, 60) or not target.wait_for(
            start + target.since(start).index(done), PROMPT, 10
        ):
            print("FAIL boot: no shell prompt")
            return 1
        print("ok   boot" + (" (procnto)" if procnto else ""))
        if not target.use_test_prompt():
            print("note: the shell didn't take the test prompt; going on with \"# \"")
        for t in tests:
            name = t.get("name", "?")
            # A QNX test needs the overlay or procnto; a native test needs
            # OsCraft; --qnx runs both.
            is_qnx = "needs" in t
            if (procnto and not is_qnx) or (not qnx and not procnto and is_qnx):
                skipped += 1
                continue
            if "gdb" in t and target.dbg_port is None:
                print(f"skip {name} (gdb: no pdebug line on QEMU's virt)")
                skipped += 1
                continue
            timed_out = []
            if "gdb" in t:
                for cmd in t["pre"]:
                    target.command(cmd, t["timeout"])
                    if target.timed_out:
                        timed_out.append(cmd)
                out = run_gdb(target, t["gdb"], max(t["timeout"], 60))
            else:
                outs = []
                for cmd in t["run"]:
                    outs.append(target.command(cmd, t["timeout"]))
                    if target.timed_out:
                        timed_out.append(cmd)
                out = "\n".join(outs)
            missing = [e for e in t["expect"] if not re.search(e, out, re.M)]
            found = [r for r in t["reject"] if re.search(r, out, re.M)]
            if missing or found or timed_out:
                failed.append(name)
                print(f"FAIL {name}")
                for cmd in timed_out:
                    print(f"       timed out after {t['timeout']:g} s: {cmd}")
                for e in missing:
                    print(f"       expected: {e}")
                for r in found:
                    print(f"       rejected: {r}")
                print("       output:\n" + "\n".join("         " + l for l in out.splitlines()[-25:]))
                if stop and name != "ipi":
                    stopped = True
                    break
            else:
                print(f"ok   {name}")
                passed += 1
    finally:
        log = target.since(0)
        target.close()
        os.makedirs(os.path.join(ROOT, "build"), exist_ok=True)
        name = "build/test-console.log"
        with open(os.path.join(ROOT, name), "wb") as f:
            f.write(log)
        # Deliberately bad requests (`hello`'s, `procmsgtest`'s) are 2-byte
        # messages, which the kernel answers without a line (TOOLS-24): a
        # note here is something to look at.
        text = log.decode(errors="replace")
        for word in ("not implemented", "panic"):
            n = text.count(word)
            if n:
                print(f"note: the console says \"{word}\" {n} time(s), see {name}")
    if stopped:
        print(f"stopped at the first failure: {passed} passed before it")
        return 1
    total = len(tests) + 1 - skipped
    print(f"{total - len(failed)}/{total} passed" + (f", {skipped} skipped" if skipped else ""))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

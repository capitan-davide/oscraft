"""The OsCraft console in QEMU, for host tools.

    con = Console.qemu("build/oscraft.ifs", log="out/console.log")
    con.wait_prompt(120)
    out = con.command("uname -a", 10)
    con.close()

Everything read is appended to the log as it arrives (raw bytes). QEMU
runs through tools/qemu/run.sh with the console (the shell) on a TCP
port. Scripts that import this run with `python3 -I`, so they add tools/ to
sys.path themselves.
"""

import os
import signal
import socket
import subprocess
import threading
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROMPT = b"\n# "


def _free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Console:
    def __init__(self, read, write, close, log):
        self._read, self._write, self._close = read, write, close
        self.log = open(log, "ab", buffering=0) if log else None
        self.buf = bytearray()
        self.lock = threading.Lock()
        self.stop = False
        threading.Thread(target=self._reader, daemon=True).start()

    @classmethod
    def qemu(cls, image, log=None):
        port = _free_port()
        env = dict(os.environ, SERIAL1=f"tcp:127.0.0.1:{port},server,wait")
        proc = subprocess.Popen([os.path.join(ROOT, "tools/qemu/run.sh"), image], env=env,
                                stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT,
                                start_new_session=True)
        for _ in range(100):
            try:
                sock = socket.create_connection(("127.0.0.1", port))
                break
            except OSError:
                time.sleep(0.1)
        else:
            os.killpg(proc.pid, signal.SIGKILL)
            raise RuntimeError("QEMU's console didn't come up")
        sock.settimeout(0.2)

        def read():
            try:
                return sock.recv(65536) or None
            except socket.timeout:
                return b""

        def close():
            sock.close()
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            proc.wait()

        return cls(read, sock.sendall, close, log)

    def _reader(self):
        while not self.stop:
            try:
                data = self._read()
            except OSError:
                return
            if data is None:
                return
            if data:
                if self.log:
                    self.log.write(data)
                with self.lock:
                    self.buf += data

    def mark(self):
        with self.lock:
            return len(self.buf)

    def since(self, mark):
        with self.lock:
            return bytes(self.buf[mark:])

    def wait_for(self, mark, pattern, secs):
        """Wait for bytes `pattern` after `mark`; True if seen."""
        end = time.time() + secs
        while time.time() < end:
            if pattern in self.since(mark):
                return True
            time.sleep(0.05)
        return False

    def send(self, text):
        self._write(text.encode())

    def wait_prompt(self, secs):
        """Get a shell prompt (the shell answers an empty line with one)."""
        end = time.time() + secs
        while time.time() < end:
            mark = self.mark()
            self.send("\r")
            if self.wait_for(mark, PROMPT, 3):
                return True
        return False

    def settle(self, quiet, secs):
        """Wait until nothing has been printed for `quiet` seconds (boot-time
        programs print on the console late); False if it never settles."""
        end = time.time() + secs
        mark = self.mark()
        since = time.time()
        while time.time() < end:
            time.sleep(0.1)
            if self.mark() != mark:
                mark, since = self.mark(), time.time()
            elif time.time() - since >= quiet:
                return True
        return False

    def command(self, cmd, secs):
        """Run a shell command; its output up to the next prompt (None if
        no prompt came back in time)."""
        mark = self.mark()
        self.send(cmd + "\r")
        done = self.wait_for(mark, PROMPT, secs)
        out = self.since(mark).decode(errors="replace").replace("\r", "")
        out = out.split("\n", 1)[1] if "\n" in out else ""
        return out.rsplit("\n# ", 1)[0] if done else None

    def close(self):
        self.stop = True
        self._close()

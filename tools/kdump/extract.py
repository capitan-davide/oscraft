#!/usr/bin/env python3
"""Extract a kernel dump written by QNX's kdumper from a console log.

Usage: extract.py <console-log> [<out>]

kdumper uuencodes the dump between "begin 644 kdump.elf[.gz]" and "end"
lines; the log may hold other output around it and CR line ends. The
last dump in the log is decoded, gunzipped if compressed (kdumper -C),
and written to <out> (default: kdump.elf), for:

    ntoaarch64-gdb <kernel ELF> -ex 'target remote | kdserver kdump.elf'
"""
import gzip
import sys


def decode_line(line):
    n = (line[0] - 32) & 63
    out = bytearray()
    for i in range(1, 1 + 4 * ((n + 2) // 3), 4):
        c = [(x - 32) & 63 for x in line[i:i + 4].ljust(4, b" ")]
        out += bytes([(c[0] << 2 | c[1] >> 4) & 255, (c[1] << 4 | c[2] >> 2) & 255, (c[2] << 6 | c[3]) & 255])
    return out[:n]


def extract(log, out):
    """Decode the last dump in console log `log` to `out`; a message, or
    an exception (ValueError) if there is no complete dump."""
    lines = [l.rstrip(b"\r") for l in open(log, "rb").read().split(b"\n")]
    begins = [i for i, l in enumerate(lines) if l.startswith(b"begin ")]
    if not begins:
        raise ValueError("no dump in the log")
    name = lines[begins[-1]].split()[-1].decode()
    data, bad = bytearray(), []
    for no, line in enumerate(lines[begins[-1] + 1:], begins[-1] + 2):
        if line == b"end":
            break
        if line:
            # Length byte, 4 characters per 3 bytes, kdumper's sequence letter.
            if len(line) - 4 * ((((line[0] - 32) & 63) + 2) // 3) not in (1, 2):
                bad.append(no)
            data += decode_line(line)
    else:
        raise ValueError("the dump is incomplete (no end line)")
    if name.endswith(".gz"):
        try:
            data = gzip.decompress(data)
        except (OSError, EOFError) as e:
            where = f"; {len(bad)} malformed lines, the first at line {bad[0]}" if bad else ""
            raise ValueError(f"the dump is damaged ({e}){where}: characters lost on the console?")
    open(out, "wb").write(data)
    return f"{out}: {len(data)} bytes (from {name})"


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__.strip().splitlines()[2])
    try:
        print(extract(sys.argv[1], sys.argv[2] if len(sys.argv) == 3 else "kdump.elf"))
    except ValueError as e:
        sys.exit(f"extract.py: {e}")


if __name__ == "__main__":
    main()

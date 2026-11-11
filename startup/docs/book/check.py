#!/usr/bin/env python3
"""Check that the book's code references match the source tree.

mdBook includes code with {{#include path:anchor}}, where the anchor is a
pair of comments in the source file:

    // ANCHOR: <name>
    ...
    // ANCHOR_END: <name>

mdBook itself does not fail when an anchor is missing: it silently
includes nothing. This script makes every reference a hard error instead:

- every {{#include}} names an existing file and an anchor that exists in
  it, with a matching ANCHOR_END and at least one line between them;
- includes by line number are rejected, since they drift with every edit;
- includes stay inside the book's own source tree (ROOT, below), so the
  book can move with it;
- every anchor in that tree is used by some chapter;
- every path written in a code span exists;
- every source file is listed in the source map;
- links to another book point to one of its chapters.

The configuration is the constants below; a book that uses this script
for another tree changes them.

Usage: python3 -I docs/book/check.py   (exit status 1 on any error)
"""

import re
import subprocess
import sys
from pathlib import Path

BOOK = Path(__file__).resolve().parent
SRC = BOOK / "src"

# ANCHOR: check_config
# The startup library's book: its tree is the repository. Paths in code
# spans are relative to it.
ROOT = BOOK.parent.parent
# Anchors of these paths (relative to ROOT) must be used by this book.
ANCHOR_SCOPE = ["src", "abi", "boards", "startup.ld", "docs/book/check.py"]
# Source files that must have a row in the source map.
SOURCE_MAP = "appendix-b-source-map.md"
SOURCES = ["src", "abi", "boards"]
# Code spans that look like paths of this tree.
PATH_SPAN = re.compile(r"`((?:src|abi|docs|boards)/[\w./-]*|startup\.ld)`")
# Links to other books, and where their chapters are (None: not checked).
OTHER_BOOK_LINK = re.compile(r"\]\(\.\./bsp/([\w-]+)\.html")
OTHER_BOOK_SRC = None
# ANCHOR_END: check_config

INCLUDE = re.compile(r"\{\{#(include|rustdoc_include)\s+([^}\s]+)\s*\}\}")
ANCHOR = re.compile(r"ANCHOR(_END)?:\s*([\w-]+)")


def anchors_in(text):
    """Map anchor name -> (start line, end line) for one source file."""
    starts, found = {}, {}
    for n, line in enumerate(text.splitlines(), 1):
        m = ANCHOR.search(line)
        if not m:
            continue
        end, name = m.group(1), m.group(2)
        if end:
            found[name] = (starts.pop(name, None), n)
        else:
            starts[name] = n
    for name, start in starts.items():
        found[name] = (start, None)
    return found


def tracked(paths):
    out = subprocess.run(["git", "ls-files", "--", *paths], cwd=ROOT,
                         capture_output=True, text=True).stdout
    return [Path(p) for p in out.split()]


def main():
    errors = []
    used = set()

    for md in sorted(SRC.rglob("*.md")):
        rel_md = md.relative_to(ROOT)
        for n, line in enumerate(md.read_text().splitlines(), 1):
            where = f"{rel_md}:{n}"
            for m in INCLUDE.finditer(line):
                spec = m.group(2)
                path, _, anchor = spec.partition(":")
                target = (md.parent / path).resolve()
                if not target.is_file():
                    errors.append(f"{where}: no such file {path}")
                    continue
                if not target.is_relative_to(ROOT):
                    errors.append(f"{where}: {path} is outside the book's tree")
                    continue
                if not anchor:
                    continue
                if re.fullmatch(r"\d*(:\d*)?", anchor):
                    errors.append(f"{where}: include by line number ({spec}); use an anchor")
                    continue
                rel = target.relative_to(ROOT)
                used.add((rel, anchor))
                start, end = anchors_in(target.read_text()).get(anchor, (None, None))
                if start is None and end is None:
                    errors.append(f"{where}: no anchor '{anchor}' in {rel}")
                elif start is None:
                    errors.append(f"{rel}:{end}: ANCHOR_END '{anchor}' without ANCHOR")
                elif end is None:
                    errors.append(f"{rel}:{start}: ANCHOR '{anchor}' without ANCHOR_END")
                elif end - start < 2:
                    errors.append(f"{rel}:{start}: anchor '{anchor}' is empty")
            for m in PATH_SPAN.finditer(line):
                path = m.group(1).rstrip("/.")
                if not (ROOT / path).exists():
                    errors.append(f"{where}: no such path {path}")
            if OTHER_BOOK_SRC is not None and OTHER_BOOK_SRC.is_dir():
                for m in OTHER_BOOK_LINK.finditer(line):
                    if m.group(1) != "index" and not (OTHER_BOOK_SRC / f"{m.group(1)}.md").is_file():
                        errors.append(f"{where}: no chapter {m.group(1)} in the other book")

    for rel in tracked(ANCHOR_SCOPE):
        if rel.suffix == ".md":
            continue
        try:
            text = (ROOT / rel).read_text()
        except UnicodeDecodeError:
            continue  # binary files (test data) hold no anchors
        for name, (start, _) in anchors_in(text).items():
            if (rel, name) not in used:
                errors.append(f"{rel}:{start}: anchor '{name}' is not used by the book")

    source_map = (SRC / SOURCE_MAP).read_text()
    for rel in tracked(SOURCES):
        if f"`{rel}`" not in source_map:
            errors.append(f"{rel}: not in the source map ({SOURCE_MAP})")

    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        print(f"{len(errors)} error(s)", file=sys.stderr)
        return 1
    print(f"{BOOK.relative_to(ROOT)}: "
          f"{len(used)} anchored listings, all resolved")
    return 0


if __name__ == "__main__":
    sys.exit(main())

#!/usr/bin/env python3
"""Check that the book's code references match the source tree.

mdBook includes code with {{#include path:anchor}}, where the anchor is a
pair of comments in the source file:

    // ANCHOR: name
    ...
    // ANCHOR_END: name

mdBook itself does not fail when an anchor is missing: it silently
includes nothing. This script makes every reference a hard error instead:

- every {{#include}} names an existing file and an anchor that exists in
  it, with a matching ANCHOR_END and at least one line between them;
- includes by line number are rejected, since they drift with every edit;
- a Rust listing covers whole items or whole blocks: its braces balance,
  and it doesn't end on the doc comment or attribute of the next item
  (a marker moved by an edit easily lands there);
- every anchor in the source tree is used by some chapter (an unused
  anchor means a listing was removed and its markers were left behind);
- every repository path written in a code span (`kernel/src/sched.rs`)
  exists;
- every source file of the kernel is listed in the
  source map (Appendix C), so a new file must find its place in the book.

Usage: python3 -I docs/book/check.py   (exit status 1 on any error)
"""

import re
import subprocess
import sys
from pathlib import Path

BOOK = Path(__file__).resolve().parent
SRC = BOOK / "src"
ROOT = BOOK.parent.parent

INCLUDE = re.compile(r"\{\{#(include|rustdoc_include)\s+([^}\s]+)\s*\}\}")
ANCHOR = re.compile(r"ANCHOR(_END)?:\s*([\w-]+)")
# A code span that looks like a path into the repository.
PATH_SPAN = re.compile(
    r"`((?:kernel|drivers|user|tools|tests|images|docs)/[\w./-]*)`"
)
# String and character literals, blanked before braces are counted.
LITERAL = re.compile(r'"(?:[^"\\]|\\.)*"|\'(?:[^\'\\]|\\.)\'')


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


def rust_shape(text, start, end):
    """What is wrong with the shape of the Rust listing between lines
    `start` and `end` (its markers): unbalanced braces, or an ending on
    the next item's doc comment or attribute."""
    body = [line for line in text.splitlines()[start:end - 1]
            if not ANCHOR.search(line)]
    depth, lowest = 0, 0
    for line in body:
        code = LITERAL.sub('""', line).split("//", 1)[0]
        for ch in code:
            if ch == "{":
                depth += 1
            elif ch == "}":
                depth -= 1
                lowest = min(lowest, depth)
    problems = []
    if depth or lowest < 0:
        problems.append("its braces don't balance")
    last = next((line.strip() for line in reversed(body) if line.strip()), "")
    if last.startswith(("///", "//!", "#[")):
        problems.append("it ends on the next item's doc comment or attribute")
    return problems


def main():
    errors = []
    used = set()

    for md in sorted(SRC.rglob("*.md")):
        rel_md = md.relative_to(ROOT)
        text = md.read_text()
        for n, line in enumerate(text.splitlines(), 1):
            where = f"{rel_md}:{n}"
            for m in INCLUDE.finditer(line):
                spec = m.group(2)
                path, _, anchor = spec.partition(":")
                target = (md.parent / path).resolve()
                if not target.is_file():
                    errors.append(f"{where}: no such file {path}")
                    continue
                if not anchor:
                    continue  # a whole file
                if re.fullmatch(r"\d*(:\d*)?", anchor):
                    errors.append(
                        f"{where}: include by line number ({spec}); "
                        "use an anchor"
                    )
                    continue
                rel = target.relative_to(ROOT)
                used.add((rel, anchor))
                start, end = anchors_in(target.read_text()).get(
                    anchor, (None, None)
                )
                if start is None and end is None:
                    errors.append(f"{where}: no anchor '{anchor}' in {rel}")
                elif start is None:
                    errors.append(f"{rel}:{end}: ANCHOR_END '{anchor}' "
                                  "without ANCHOR")
                elif end is None:
                    errors.append(f"{rel}:{start}: ANCHOR '{anchor}' "
                                  "without ANCHOR_END")
                elif end - start < 2:
                    errors.append(f"{rel}:{start}: anchor '{anchor}' "
                                  "is empty")
                elif rel.suffix == ".rs":
                    for problem in rust_shape(target.read_text(), start, end):
                        errors.append(f"{rel}:{end}: anchor '{anchor}': "
                                      f"{problem}")
            for m in PATH_SPAN.finditer(line):
                path = m.group(1).rstrip("/.")
                if "*" in path or "NNNN" in path:
                    continue
                if not (ROOT / path).exists():
                    errors.append(f"{where}: no such path {path}")

    # Anchors in tracked files that no chapter uses. Markdown files only
    # describe anchors, they never hold one. The startup library's anchors
    # belong to its own book (startup/docs/book), checked by its own check.py.
    files = subprocess.run(
        ["git", "grep", "-l", "-E", "ANCHOR(_END)?:", "--", ".",
         ":!docs/book", ":!startup", ":!*.md"],
        cwd=ROOT, capture_output=True, text=True,
    ).stdout.split()
    for f in files:
        rel = Path(f)
        for name, (start, _) in anchors_in((ROOT / rel).read_text()).items():
            if (rel, name) not in used:
                errors.append(f"{rel}:{start}: anchor '{name}' is not "
                              "used by the book")

    # Every source file has its row in the source map (Appendix C).
    source_map = (SRC / "appendix-c-source-map.md").read_text()
    sources = subprocess.run(
        ["git", "ls-files", "kernel/src"],
        cwd=ROOT, capture_output=True, text=True,
    ).stdout.split()
    for f in sources:
        if f"`{f}`" not in source_map:
            errors.append(f"{f}: not in the source map "
                          "(docs/book/src/appendix-c-source-map.md)")

    for e in errors:
        print(e, file=sys.stderr)
    if errors:
        print(f"{len(errors)} error(s)", file=sys.stderr)
        return 1
    print(f"book: {len(used)} anchored listings, all resolved")
    return 0


if __name__ == "__main__":
    sys.exit(main())

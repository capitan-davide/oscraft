<!-- Title: start with the issue IDs, e.g. "SCHED-4, SCHED-5: ...". -->

Closes #

## What changed

## Measured

<!-- For the native personality: FreeBSD's semantics, and how they were checked.
     For the QNX personality: the probe (tools/qnx/probe.py --diff ...). -->

| | reference | OsCraft before | OsCraft after |
|---|---|---|---|
| | | | |

## Checks

- [ ] `make test` (dev kernel): N/N passed, on `<sha>`
- [ ] `make test PROFILE=release`: N/N passed, on `<sha>`
- [ ] `make test QNX=1` (if the QNX personality is touched): N/N passed
- [ ] `make clippy`: no new warnings
- [ ] `make -C docs/book check`, and the book, the notes and the skills say what the code now does: pages updated, or "no documented behaviour changed"
- [ ] A check in a `tests/native/*.c` or `tests/qnx/*test.c` program, wired into `tests/target.tests`
- [ ] Commit messages end with `Co-Authored-By:` for the agent that wrote them

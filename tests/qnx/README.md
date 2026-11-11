# QNX test programs

C programs built with the QNX SDP's GCC (`$QNX_HOST/usr/bin/ntoaarch64-gcc`,
called directly) against QNX's own libc (`$QNX_TARGET`): source the SDP's
`qnxsdp-env.sh` first, wherever the SDP is installed. They run on OsCraft
to check binary compatibility and to show what QNX's libc
and dynamic loader need from the kernel and procnto.

`make qnx-tests` builds them into `build/qnx-tests/`; `make image` packs
them when present.

How they are run:

- `tests/target.tests` lists the target suite: each test runs one of these
  programs at the shell of the booted image and matches its output against
  procnto's measured results (format in `tools/test/run-tests.py`).
  `make test QNX=1` runs it in QEMU, `make qnx-test` on QNX's own kernel.
- `tools/qnx/probe.py prog.c` builds one program, adds it to both QNX's
  reference image and OsCraft's, and prints both outputs (`--diff` for a
  diff): how a behaviour is measured on procnto before it becomes a test
  here.

# OsCraft

A microkernel RTOS written in Rust for AArch64, with FreeBSD's userland
as its own. It runs on QEMU's `virt` machine.

- **Native: FreeBSD.** FreeBSD's libc, libthr, rtld, sh and utilities,
  built from source for `aarch64-unknown-freebsd` with upstream clang
  and Rust, over libsys, a Rust layer that turns FreeBSD's system calls
  into OsCraft's kernel calls and messages. A program for OsCraft is a
  FreeBSD program; `make sdk` packs the sysroot and toolchain files.
  FreeBSD's network stack runs in a server, `netd`.
- **Optional: QNX 8.0 compatibility.** The kernel's call and message ABI
  is binary-compatible with QNX 8.0's procnto, so QNX programs run
  unmodified as a second-class personality, under `/compat/qnx`. This is
  opt-in: `QNX=1` adds QNX's own programs to the image from the user's
  licensed QNX SDP, and the suite's QNX tests, measured against procnto.
  Without the SDP nothing of QNX is built or needed.

## Quick start

Needs Rust (via rustup; `rust-toolchain.toml` selects the version, and a
nightly for libsys), clang, ld.lld and `qemu-system-aarch64`, and
FreeBSD's source set up once (`make freebsd-bootstrap`). `docs/setup.md`
has the whole recipe and `make doctor` checks the host.

```
make doctor   # what this host is missing
make freebsd-bootstrap; make libsys freebsd-libs freebsd-progs freebsd-terminfo   # the FreeBSD userland, once
make run      # build the kernel and boot image, run it in QEMU (Ctrl-A X quits)
make debug    # same, waiting for gdb on :1234
make test     # boot the image in QEMU and run the native tests (tests/target.tests)
make sdk      # the SDK: the sysroot and toolchain files, build/sdk/*.tar.zst
```

With the QNX SDP 8.0 (source its `qnxsdp-env.sh` in bash first):

```
make test QNX=1   # the image with the QNX overlay, and the QNX tests too
make qnx-test     # the QNX tests on QNX's own kernel, in the same QEMU machine
python3 -I tools/qnx/probe.py --diff prog.c   # one C program on procnto and on OsCraft
```

## Documentation

Documentation lives in [`docs/`](docs/):

- [`docs/book/`](docs/book/) - a book explaining the kernel, with its code.
- [`startup/docs/book/`](startup/docs/book/) - the startup library's book.

[`AGENTS.md`](AGENTS.md) and [`CONTRIBUTING.md`](CONTRIBUTING.md) say how
work is done and shared.

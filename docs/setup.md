# Setting up a development host

What a machine needs to build OsCraft, its FreeBSD userland and the
image, and to run the test suite in QEMU. `make doctor` checks all of
it and says what is missing.

## What is built from where

| Part | Source | Built with |
|---|---|---|
| The kernel | this repository, `kernel/` | Rust, the toolchain pinned by `rust-toolchain.toml`, target `aarch64-unknown-none-softfloat` |
| The startup, for QEMU's virt | `startup/` (the library and `boards/qemu-virt`) | the same Rust, partially linked with ld.lld |
| libsys, the system-call layer of FreeBSD's libc on OsCraft | `user/libsys`, `user/proto` | Rust nightly (`LIBSYS_TOOLCHAIN` in the Makefile) with `rust-src`, because `aarch64-unknown-freebsd` has no prebuilt `core` |
| FreeBSD's userland: libc, libthr, librt, rtld, sh and the utilities | the FreeBSD source tree, stable/15 at the commit in `tools/freebsd/UPSTREAM_COMMIT`, with the patches in `tools/freebsd/patches/` | host clang and lld through FreeBSD's own `tools/build/make.py` |
| The native servers (pipe, random, pty, tmpfs, syslogd, diskd, netd, usbd, pcid) and their FreeBSD kernel code | `user/`, and FreeBSD's `sys/` for netd, usbd and pcid | the nightly with `-Zbuild-std`, linked by clang against the sysroot; FreeBSD's `config(8)` and build environment for the C |
| The native test programs | `tests/native/` | clang against the sysroot |
| The boot image | `images/qemu-virt.build` (the startup, the kernel, the native servers, FreeBSD's userland, FreeBSD's sh on the kernel's console); for `make test`, with `tests/native/image.build` | the clean-room mkifs in `tools/mkifs` |
| Optional: the QNX overlay, the QNX test programs, the QNX reference image | `images/qnx.build`, `tests/qnx/` (its `image.build` and `reference.build`) | the QNX SDP 8.0's programs, libraries and GCC (`QNX=1`) |

## Host packages

Linux. `clang`, `lld`, `qemu-system-aarch64`, `python3`, `git`, `tic`
(ncurses), `mkfs.fat` (dosfstools), `mcopy` (mtools), `ssh-keygen`, and
optionally `bc` (bmake's self-test wants it; the bootstrap takes the
build as tested when the self-test fails). Rust through rustup: the
kernel's toolchain with the target `aarch64-unknown-none-softfloat`, and
the nightly named by `LIBSYS_TOOLCHAIN` with the `rust-src` component:

```sh
rustup target add aarch64-unknown-none-softfloat
rustup toolchain install nightly-2026-08-17 --component rust-src
```

## FreeBSD's source, once per machine

```sh
make freebsd-bootstrap
```

runs `tools/freebsd/bootstrap.sh`: a partial clone of OsCraft's fork of
FreeBSD, <https://github.com/capitan-davide/freebsd-src>, branch
`oscraft`, into `../freebsd-src` (stable/15 at the commit in
`tools/freebsd/UPSTREAM_COMMIT` plus OsCraft's commits: the libsys hook
in libc and rtld, build fixes, the configurations of the kernel code the
servers take); bmake bootstrapped into `../freebsd-obj` as make.py
expects it; then the cross-build's stage targets, one at a time, which
leave a sysroot at `../freebsd-obj<src>/arm64.aarch64/tmp` with the
headers. This takes a while the first time and is skipped afterwards.
`FREEBSD_SRC` and `FREEBSD_OBJ` in the Makefile name the two directories.

`tools/freebsd/patches/` holds the fork's commits as patches, the same
difference in a form that needs no fork: with
`OSCRAFT_FREEBSD_GIT=https://git.freebsd.org/src.git` the bootstrap clones
upstream and applies them. To change FreeBSD, commit in `../freebsd-src`
on `oscraft`, push to the fork, and regenerate the patches with
`git format-patch <upstream commit>..oscraft -o tools/freebsd/patches/`.

## Building

In order:

```sh
make libsys              # the libsys object, build/freebsd/oscraft_libsys.o
make freebsd-libs        # csu, compiler_rt, libsys, libc, rtld, libgcc, libthr, librt, libutil... into the sysroot
make freebsd-progs       # the support libraries, sh and the utilities (FREEBSD_PROGS)
make freebsd-terminfo    # the terminal entries
make image               # the kernel, the startup, the servers, the test programs and the image, build/oscraft.ifs
make test                # the native suite in QEMU
```

`make run` boots the image in QEMU with the console on the terminal.
After a change to libsys, `make freebsd-libs freebsd-progs` again: the
object is linked into libc, libsys.so, rtld and the static programs.
After a change to the kernel, a server or the tests, `make image` is
enough.

## The QNX SDP, optional

OsCraft's kernel is binary-compatible with QNX 8.0, and the QNX
personality is measured against QNX's own kernel. Both need QNX's
programs, which come only from the SDP 8.0, from QNX Software Center
under the user's own licence (the non-commercial one is free). Nothing
of it is needed to build, run or test the system itself.

Install the SDP anywhere, and before building with `QNX=1` or using the
`qnx-*` targets and `tools/qnx/probe.py`, source its environment in bash
(its script refuses other shells):

```sh
bash
source "$SDP/qnxsdp-env.sh"      # sets QNX_HOST and QNX_TARGET
```

Nothing from the SDP is in the repository or may be written into it: the
build reaches it only through `QNX_HOST` and `QNX_TARGET`.

```sh
make test QNX=1          # the image with the QNX overlay (images/qnx.build): QNX's
                         # shell on its serial driver as the console, the QNX tests too
make qnx-test            # the QNX tests on QNX's own kernel (tests/qnx/reference.build)
make qnx-run             # boot QNX's kernel in QEMU
```

The kernel's crash dumps (`KDUMP=1`, `tools/kdump`) and traces
(`tools/trace`) use QNX's kdumper and tracelogger, so they need the SDP
too.

## Where things are on the target

FreeBSD's programs and libraries are at `/bin`, `/usr/bin`, `/lib` and
`/usr/lib`, and, with the overlay, QNX's under `/compat/qnx`. A QNX
program looks up paths there first, as a Linux program on FreeBSD does in
`/compat/linux`: its `/bin/sh` is `/compat/qnx/bin/sh`, its `/dev/null`
the system's. So `PATH` needs no QNX directories, and each personality
finds its own tools; a native program reaches QNX's by their full path.
Everything in the image is also under `/proc/boot`.

## The SDK

`make sdk` assembles the sysroot's headers, libraries and loader with the
toolchain files of `tools/sdk/` (a clang config, `oscraft-cc` wrappers,
a CMake toolchain file, a Cargo configuration) and a manifest into
`build/sdk/oscraft-sdk-<version>-aarch64.tar.zst`; `make sdk-check`
builds a C program through CMake and a Rust program through Cargo
against it, out of tree, and runs both in QEMU. `tools/sdk/README.md`
is the SDK's own guide.

## Programs against the sysroot

A C program for OsCraft is a FreeBSD program:

```sh
clang --target=aarch64-unknown-freebsd15.0 --sysroot=$SYSROOT -fuse-ld=lld -o prog prog.c
```

as the Makefile builds the test programs under `tests/native/`, static
or dynamic. The calls libsys does not implement yet print
`libsys: not implemented: <name>` on standard error once and fail with
ENOSYS; `grep not_implemented user/libsys/src/stubs.rs` lists them.

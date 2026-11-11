# FreeBSD's userland for OsCraft

OsCraft's native personality is FreeBSD's: its libc, libthr, libc++ and
dynamic loader, built from FreeBSD's own source tree with upstream clang,
over `user/libsys`, OsCraft's replacement for FreeBSD's system-call layer
(kernel calls and messages to servers instead of traps). The target
triple stays `aarch64-unknown-freebsd`, so no toolchain is patched.

## The tree

A fork of FreeBSD `stable/15`, branch `oscraft`, checked out next to this
repository as `../freebsd-src` (`FREEBSD_SRC`), with objects in
`../freebsd-obj` (`FREEBSD_OBJ`). The branch carries twelve commits,
kept as patches in `patches/`:

- 0001, 0003, 0004: `lib/libsys/Makefile.sys` and
  `libexec/rtld-elf/rtld-libc/Makefile.inc` take OsCraft's one object
  (`user/libsys`) when `OSCRAFT_LIBSYS_OBJ` is set;
- 0002, 0005: two fixes for cross-building on a Linux host (mtree's
  include order, a warning in libgcc_eh under a newer host clang);
- 0007, 0010: what FreeBSD's kernel code needs to run in a process over
  bsdkern: `curthread` and the per-CPU data per thread, and no priority
  lending in `epoch`;
- 0006, 0008, 0009, 0011, 0012: a kernel configuration `OSCRAFT_NET` in
  the fork. The build no longer reads it: the configurations are in this
  repository, `tools/freebsd/conf/OSCRAFT_*`.

## Building, once

FreeBSD's `tools/build/make.py` bootstraps bmake and cross-builds from
Linux. The stages before the libraries are run once by hand, one target
at a time (bmake runs several command-line targets in parallel):

```sh
export MAKEOBJDIRPREFIX=$FREEBSD_OBJ PATH=$FREEBSD_OBJ/hostbin:$PATH
cd $FREEBSD_SRC
for t in _worldtmp _legacy _bootstrap-tools _obj _build-tools _cross-tools _build-metadata _includes; do
    python3 tools/build/make.py --cross-bindir=/usr/bin TARGET=arm64 TARGET_ARCH=aarch64 -j16 \
        -DWITHOUT_CLEAN -DWITHOUT_TESTS -DWITHOUT_PROFILE -DWITHOUT_MAN -DWITHOUT_LIB32 -DWITHOUT_DEBUG_FILES $t
done
python3 tools/build/make.py ... buildenv BUILDENV_SHELL="sh -c 'cd lib/libc++ && make ... includes'"
```

Host quirks met on this machine: bmake's self-test wants `bc` (bootstrap
it by hand with the test skipped and write `bmake-install/.make-py-config`
so make.py keeps it), `time(1)` is absent (a shim in `hostbin`), and the
libc++ headers go into the sysroot with `make includes` in `lib/libc++`.

Then, from this repository, as often as libsys changes:

```sh
make libsys          # user/libsys as one object, build/freebsd/oscraft_libsys.o
make freebsd-libs    # csu, compiler_rt, libsys, libc, rtld, libgcc, libthr, into the sysroot
make native-tests    # tests/native's programs, linked against the sysroot
make test TESTS="hello-nolibc libc dyn thr"
```

`make libsys-stubs` regenerates `user/libsys/src/stubs.rs` from the
tree's `sys/sys/syscall.mk`: every call libsys does not implement by
hand reports itself on the standard error and fails with ENOSYS, so a
program's first missing call is visible.

# The OsCraft SDK

Programs for OsCraft are FreeBSD programs for aarch64: FreeBSD's libc,
libthr, librt and the rest, built against OsCraft's kernel. This SDK is
that sysroot (`sysroot/`: headers, libraries, the CSU objects, rtld) and
the configuration that points the host's own clang, lld, CMake and Cargo
at it. It ships no compiler.

- `bin/oscraft-cc`, `bin/oscraft-c++`: clang and clang++ with `clang.cfg`.
  `oscraft-cc -o prog prog.c` builds a dynamic program; `-static` a
  static one.
- `oscraft.cmake`: `cmake -DCMAKE_TOOLCHAIN_FILE=<sdk>/oscraft.cmake`.
- `cargo-config.toml`: the Cargo settings for `aarch64-unknown-freebsd`
  (nightly with `rust-src`; std is built from source, the target has no
  prebuilt one).
- `MANIFEST`: the kernel and FreeBSD commits this SDK was built from.

Needs on the host: clang and lld (any recent version), and for Rust a
nightly with the `rust-src` component. Running a program needs an
OsCraft image with it: the kernel repository's `make image` with the
program listed in `images/qemu-virt.build`, or an image of your own with
`tools/mkifs`.

Calls the C library makes that OsCraft does not serve yet print
`libsys: not implemented: <name>` once on standard error and fail with
ENOSYS.

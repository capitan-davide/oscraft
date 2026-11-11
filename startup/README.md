# oscraft-startup

A startup library in Rust, compatible with QNX OS 8.0: the generic part of
a startup program, the counterpart of QNX's `libstartup.a`. A board crate adds the
board's facts and code, and boots QNX's own kernel.

It builds the system page, copies and patches the kernel callouts, sets
up the AArch64 page tables and the GICv2, loads the bootstrap
executables, starts the secondary CPUs and hands over to the kernel. It
has no board code. The first board to use it is the Raspberry Pi 4, in
[oscraft-bsp-raspberrypi-bcm2711-rpi4](https://github.com/capitan-davide/oscraft-bsp-raspberrypi-bcm2711-rpi4).

- `src/`: the library.
- `abi/`: the layouts of QNX's C structures, recorded from the SDP's
  headers, which the unit tests check the Rust types against.
- `startup.ld`: the partial-link script for the startup program mkifs
  takes (see the book, chapter 2).
- `docs/book/`: the library's book, published at
  <https://capitan-davide.github.io/oscraft-startup/>.

## Using it

A board crate depends on it as a git dependency:

```toml
[dependencies]
oscraft-startup = { git = "https://github.com/capitan-davide/oscraft-startup" }
```

and is built as a static library for `aarch64-unknown-none-softfloat`,
then partially linked with `startup.ld` (`ld.lld -r -u _start -T
startup.ld`). The book's chapter 14 describes a board crate.

## Tests and the book

```text
make test         # unit tests, on the host
make check        # clippy, for the target
make book         # checks the book's references to the code, builds it in docs/book/build
make abi-layout   # regenerates abi/layout.txt (needs the QNX SDP's environment)
```

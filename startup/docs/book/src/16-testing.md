# Testing

Startup is hard to debug: it runs before anything that could help, and
when it gets the contract wrong, the kernel usually fails much later, far
from the mistake. So the library is tested in three ways, from the
cheapest to the most complete: its binary interface against QNX's
headers, its logic on the host, and whole boots against QNX's own startup.

![How startup is tested](images/ch16-tests.svg)

*Figure 16.1: The four kinds of tests, each catching what the others cannot.*

## The binary interface

Every structure in `src/abi` must have exactly the layout of its C
counterpart; a wrong offset in the system page would only show as a
kernel misbehaving. The layouts are checked mechanically, without
running anything on the target.

`abi/layout.c` records sizes and offsets from the SDP's headers. It
cannot print them, since it is compiled for AArch64, so it uses a trick:
each value becomes the size of an array (Listing 16.1), which the
compiler records in the object file's symbol table.

*Listing 16.1: Recording a layout as array sizes (`abi/layout.c`).*

```c
{{#include ../../../abi/layout.c:layout_macros}}
```

`abi/layout.py` compiles nothing itself: it reads the sizes back with the
SDP's `nm -S` and writes them, one per line, to `abi/layout.txt`, which is
in the repository. A unit test (`src/abi/layout_tests.rs`) computes the
same values for the Rust types with `offset_of!` and `size_of`, and
compares: every C value must match, and every Rust value must be checked.

So the test needs no SDP. Only regenerating `layout.txt` does, after a
change to the headers or to the list of checked fields.

> [!NOTE]
> `__FLEXARY` structures (with a variable-size array at the end, like
> `struct image_header`) have one element of the array in their `sizeof`.
> The Rust types stop where the array starts, so `layout.c` records the
> array's offset as their size.

## Logic on the host

Most of the library is plain computation: parsing the command line and
the options, range arithmetic, laying out the system page, the asinfo
tree, hwinfo encoding, ELF segment plans, timer arithmetic, patching
instructions, the device tree reader, decompression. These parts do not touch hardware, and they are
unit-tested on the host with `cargo test`. The decompressor's test uses
real data: the last blocks of an image file system that mkifs compressed
(`src/testdata/ucl.bin`), whose decompressed length and hash come from
what the SDP's `dumpifs` decompressed them to. The modules that do touch
hardware (`src/arch/aarch64`, `src/startup.rs`, `src/ifs.rs`) are only
built for AArch64.

## Booting against QNX's startup

The final test is a boot. Startup either reaches a shell or it does not,
but a boot also allows a much finer comparison: the same image can be
built with QNX's own startup for the same board, and both can be run in
an emulator. The kernel's view of the system page is then available on
both systems with `pidin syspage`, and a `diff` shows every difference
in the contract:

```text
$ diff <(sed -E 's/[0-9a-f]{16}/X/g' qnx.txt) <(sed -E 's/[0-9a-f]{16}/X/g' ours.txt)
```

Masking 16-digit hex values hides the addresses, which differ for good
reasons (the sections are in a different order). What remains are real
differences, and each one is a decision to check. Running QNX's startup
with `-vvvv` also makes it print its system page and its view of the
CPUs before it starts the kernel, which helps when the kernel does not
come up at all.

On `virt`, QNX's own startup for the machine (`startup-qemu-virt`, in
the SDP) is the reference to compare with: the same build file, with one
startup or the other.

## The book

The book itself is tested too. Its listings come from named anchors in
the source, and `docs/book/check.py` checks, among other things, that every
anchor a chapter names exists, that every anchor in the source is used,
that the listings come from the crate itself, and that every source file
appears in the source map. Its configuration (Listing 16.2) says what the
book's tree is.

*Listing 16.2: The book checker's configuration (`docs/book/check.py`).*

```python
{{#include ../check.py:check_config}}
```

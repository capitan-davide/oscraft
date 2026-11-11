# Preface

> [!NOTE]
> This book is a draft, reworked around its new reference board, QEMU's
> `virt`. Its details will be filled in as that board's BSP is completed.

This book explains `oscraft-startup`, a library in Rust for writing
*startup programs* compatible with QNX OS 8.0. It is not a QNX product: it
implements the interface QNX's kernel expects, so that a board can boot
QNX's own kernel with it. Startup is the first program
that runs from a QNX boot image. It finds out what the board is,
describes it to the kernel in a structure called the *system page*, sets
up the MMU and the CPUs, and starts the kernel. QNX writes its startup
programs in C, on top of a library of its own, `libstartup.a`.
`oscraft-startup` takes that library's place: a board crate written with
it boots QNX's own, unmodified kernel, and OsCraft's, which keeps the
same contract. The library is part of OsCraft's repository (`startup/`).

The book is for readers who know systems programming and some Rust, but
not necessarily QNX or the AArch64 architecture. Each topic first
explains the concept and its purpose (exception levels, translation
tables, interrupt controllers, the system page), then how the library
solves it. Diagrams carry most of the explanation, with the real numbers
of one machine; code listings appear where the code itself says something
the prose cannot, and are taken from the source when the book is built.

That machine is QEMU's `virt`, the library's reference board (Chapter 2):
every address, interrupt and size in the book is `virt`'s, and its board
crate, `boards/qemu-virt`, is the example of Chapter 15. Boards outside
OsCraft's tree have books of their own.

## How the book is organized

**Part I** is background: what startup does and why it exists, the
reference board, and what the boot image that holds startup looks like.

**Part II** follows the library in the order a boot runs: the first
instructions, the context a board works with, physical memory, the system
page and its sections, the callouts, the interrupt controller, the CPUs
and timer, the kernel's address space, loading the kernel, starting the
other CPUs, and the jump into the kernel.

**Part III** is about using the library: writing a board crate, and
testing startup without and with hardware.

The appendices list the command-line options, map every source file to
the chapter that explains it, and define the terms.

## Conventions

- Paths are relative to the library's root (`startup/` in OsCraft's
  tree): `src/startup.rs`, `boards/qemu-virt/src/board.rs`.
- *Listings* are code from the source; *figures* are diagrams; both are
  numbered per chapter.
- QNX's C names (`struct syspage_entry`, `init_smp()`) appear where a
  Rust item mirrors them, so that QNX's documentation can be read
  alongside.

> [!NOTE]
> Notes give background that is useful but not needed to follow the text.

> [!WARNING]
> Warnings point out things that are easy to get wrong when changing the
> code.

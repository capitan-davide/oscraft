# Appendix A: Kernel Call Numbers

A kernel call is `svc #0x51` with its number in `w8` (Chapter 7). The
kernel defines the numbers it handles in `kernel/src/ker/number.rs`,
shown here in full: the calls of the microkernel's own interface, and
the three added for FreeBSD's thread library (`thr_new`, `thr_self` and
the futex, Chapter 23). The numbering of the first group is that of QNX
OS 8.0's kernel, which is what lets the optional QNX personality run
QNX programs unmodified (Chapter 32); native programs never see the
numbers, since they call libsys (Chapter 24). `ker::dispatch()` maps
each number to its handler, and Table 7.1 groups the calls. A number not
listed returns `ENOSYS`.

*Listing A.1: The kernel-call numbers (`kernel/src/ker/number.rs`).*

```rust
{{#include ../../../kernel/src/ker/number.rs}}
```

`__MUON_CALL_INTERRUPT_WAIT` (256) is outside the table: it is a short
form of `InterruptWait()` that takes only the unmask flag (Chapter 15).

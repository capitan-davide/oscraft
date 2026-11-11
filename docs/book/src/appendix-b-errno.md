# Appendix B: Error Numbers

Kernel calls and the replies of process 1 and the servers report errors
with FreeBSD's error numbers, those of its `<sys/errno.h>`: OsCraft's
native numbers (Chapter 23). A failing kernel call returns one in `x0`,
one instruction after the normal return point (Chapter 7); a server's
reply carries one as its status (Chapter 10). The numbers themselves are
in `user/proto/src/errno.rs`; the kernel names the ones it uses in
`kernel/src/ker/errno.rs`, shown here in full.

A few errors exist only in QNX's numbering. The kernel marks them
(`QNX_ONLY`): a QNX process receives QNX's number, and a native process
receives `EIO`. For a QNX process every number is converted at the
kernel's boundary (Chapter 32).

*Listing B.1: The error numbers (`kernel/src/ker/errno.rs`).*

```rust
{{#include ../../../kernel/src/ker/errno.rs}}
```

Programs use the C library's names, so the values only matter when
reading raw traces or dumps: `ENOSYS` is 78 and `ETIMEDOUT` is 60, as on
FreeBSD.

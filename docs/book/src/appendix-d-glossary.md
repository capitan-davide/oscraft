# Appendix D: Glossary

**ABI.** Application binary interface: everything a compiled program
depends on bit for bit, from structure layouts and error numbers to how a
system call is made. OsCraft's native ABI is FreeBSD's (Chapter 23).

**Ability.** A privilege for one kind of operation, in a root and a
non-root domain per process (Chapter 17).

**ASID.** Address-space identifier: tags TLB entries with their process,
so that switching processes needs no TLB flush (Chapter 16).

**Boot script.** The commands the buildfile's `[+script]` section lists,
run by process 1 after boot (Chapter 19).

**Bootstrap executable.** A program in the boot image's `.boot` section,
run in place before or as the kernel: startup, `kdumper`, the kernel
(Chapter 5).

**bsdkern.** The layer that gives FreeBSD's kernel code, running inside a
server, the kernel environment it expects: threads, locks, memory,
timers, the device tree of drivers (Chapter 27).

**Buildfile.** The text file that says what goes into a boot image: the
programs it boots with, the boot script and the files
(`images/qemu-virt.build`, Chapter 3).

**Callout.** A small board-specific routine that startup puts in the
system page: console output, reboot, IPIs, interrupt-controller code
(Chapter 5).

**Channel.** What a server receives messages on (Chapter 10).

**Client bus.** The PCI bus as a server that holds drivers sees it: its
devices and their configuration space reached through `pcid` by message
(Chapter 29).

**Connection (coid).** A client's link to a channel; a file descriptor is
one (Chapter 10).

**EL0, EL1.** AArch64 exception levels: user mode and kernel mode
(Chapter 6).

**Event (`sigevent`).** A description of how to notify a thread: pulse,
signal, new thread, memory write, semaphore post or interrupt wake
(Chapter 11).

**Family (of timers).** The set of CPUs whose timeouts and timers one CPU
expires on its tick (Chapter 13).

**Futex.** A wait on the value of a word of user memory, in the kernel:
what libsys builds FreeBSD's `_umtx_op()` on (Chapters 9 and 23).

**Held request.** A message process 1 (or any server) received and does
not answer yet, keeping its receive ID (Chapter 18).

**Image filesystem (IFS).** The boot image's files, served read-only by
process 1 at `/` and `/proc/boot` (Chapters 5, 22).

**IPI.** Interprocessor interrupt: one CPU interrupting another, to
reschedule or to look at its timers (Chapter 8).

**Kernel thread.** A thread of process 1, running at EL1 (Chapter 8).

**libsys.** The layer under FreeBSD's C library where each system call
is made of OsCraft kernel calls and messages (Chapter 24).

**Link reply.** The path manager's answer to an open: the servers to
try, deepest prefix first (Chapter 21).

**Logical vector.** An interrupt's number as the kernel sees it: a
controller's base plus its local number (Chapter 15).

**Memory object.** Memory that belongs to no one address space, such as a
`/dev/shmem` object (Chapters 16, 20).

**Module.** FreeBSD kernel code built as a shared object and loaded into
a server at run time with `kldload`: a driver, a firewall (Chapters 27
and 28).

**Mount.** A server attached at a prefix of the path space (Chapter 21).

**Native.** Of the system's own interface, FreeBSD's. A native process
gets FreeBSD's structures and numbers with no conversion (Chapter 23).

**OCB.** Open-file context: a server's state for one open file
(Chapter 22).

**Owner value.** A thread's `((pid & 0x7fff) << 16) | tid`, stored in the
mutexes it holds (Chapter 9).

**Personality.** Which interface the kernel presents to a process:
native, or QNX's, converted at the kernel's boundary. Chosen from the
program file when it is loaded (Chapters 23 and 32).

**Prefix.** The part of the path name space a server registered with
the pathname manager (Chapter 21).

**Priority inheritance.** Running a server or a mutex owner at the
priority of whoever waits for it (Chapters 9, 10).

**Process 1.** The kernel's own process, whose threads implement the
process, memory and pathname managers (Part III).

**Pulse.** A small, non-blocking message on a channel (Chapters 10, 11).

**qnxd.** The optional service that translates a QNX program's messages
for the native servers (Chapter 33).

**Receive ID (rcvid).** The handle a server gets for a received message:
the client thread's ID (Chapter 10).

**Recursive mapping.** A translation-table entry that points to its own
table, making all tables reachable at computed addresses (Chapter 16).

**Resource manager.** A server that presents what it manages as files
under a path prefix, answering open and I/O messages (Chapters 22 and
25).

**scoid.** Server connection ID: how a server knows a client process's
connections to one of its channels (Chapter 10).

**Side channel.** A connection ID from `0x40000000` up: not a file
descriptor (Chapter 10).

**Startup.** The program that runs before the kernel, finds the
machine's memory, processors and devices, and writes the system page
(Chapter 5).

**Superlocked memory.** Memory backed and locked when mapped, never
faulted in later (Chapter 16).

**Sysroot.** The headers and libraries a compiler needs to build
programs for a system; OsCraft's is FreeBSD's, built from its source
(Chapter 26).

**System page.** The hardware description startup builds for the kernel
(Chapter 5).

**Tick.** The clock's periodic interrupt, 1 ms by default, on one grid
for all CPUs (Chapter 13).

**Trap frame.** A thread's user registers, saved at the top of its kernel
stack on every exception (Chapter 6).

**virtio.** A family of devices designed for virtual machines, whose
driver and device exchange buffers through shared queues; QEMU's network
device on the `virt` machine is one (Chapter 28).

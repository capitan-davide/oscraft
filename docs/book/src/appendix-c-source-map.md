# Appendix C: Source Map

Every source file of the kernel, what it contains, and the chapter that
explains it; then the rest of the tree, by directory. The book's check
(`make -C docs/book check`) fails when a file of the kernel is missing
here, so a new file must find its place in the book. A dash marks a file
no chapter covers yet.

## The kernel core

*Table C.1: The kernel core.*

| File | Contents | Chapter |
|---|---|---|
| `kernel/kernel.ld` | the kernel's linker script: where its parts are placed | 5 |
| `kernel/src/bootargs.rs` | the kernel's command line | 5 |
| `kernel/src/clock.rs` | the tick, tickless idle, the monotonic clock | 13 |
| `kernel/src/console.rs` | `kprintln!` through the debug callout | 5 |
| `kernel/src/msgbuf.rs` | the message buffer: `/dev/klog`, `dmesg` | 22, 36 |
| `kernel/src/interrupt.rs` | interrupt controllers and dispatch | 15 |
| `kernel/src/ipi.rs` | interprocessor interrupts | 8 |
| `kernel/src/kdebug.rs` | handing a crash to the kernel dumper | 36 |
| `kernel/src/main.rs` | `kmain()`, `kmain_ap()`: bringing up the kernel and each CPU | 5 |
| `kernel/src/percpu.rs` | per-CPU data | 6 |
| `kernel/src/report.rs` | the boot report of the system page | 5 |
| `kernel/src/sched.rs` | threads, the ready queue, `schedule()`, blocking and waking | 8 |
| `kernel/src/selftest.rs` | boot self-tests | 36 |
| `kernel/src/smp.rs` | releasing the other CPUs | 5 |
| `kernel/src/sporadic.rs` | sporadic scheduling budgets | 8 |
| `kernel/src/sync.rs` | `SpinLock`, `IrqSpinLock`, `InitOnce` | 4 |
| `kernel/src/syspage.rs` | the system page's structures and accessors | 5 |
| `kernel/src/timeout.rs` | timeouts and sleeps | 13 |

## The AArch64 layer

*Table C.2: The AArch64 layer.*

| File | Contents | Chapter |
|---|---|---|
| `kernel/src/arch/aarch64/boot.s` | `_start`, `ap_start`, the bootstrap header and data | 5 |
| `kernel/src/arch/aarch64/context.rs` | kernel context, FP/SIMD state | 6 |
| `kernel/src/arch/aarch64/context.s` | `cpu_switch`, thread start, FP save and restore | 6 |
| `kernel/src/arch/aarch64/exception.rs` | `TrapFrame`, classifying exceptions, fault reports | 6 |
| `kernel/src/arch/aarch64/intr_bridge.s` | calling a code fragment | 15 |
| `kernel/src/arch/aarch64/intrgen.rs` | copying the interrupt controller's code fragments | 15 |
| `kernel/src/arch/aarch64/kercall.rs` | kernel calls from kernel threads | 6 |
| `kernel/src/arch/aarch64/mmu.rs` | the kernel's translation tables | 16 |
| `kernel/src/arch/aarch64/mod.rs` | IRQ masking, per-CPU register, wait for interrupt | 6 |
| `kernel/src/arch/aarch64/timer.rs` | the generic timer | 13 |
| `kernel/src/arch/aarch64/uspace.rs` | user address spaces | 16 |
| `kernel/src/arch/aarch64/vectors.s` | the exception vectors, saving and restoring the trap frame | 6 |
| `kernel/src/arch/mod.rs` | the architecture module | 6 |

## Kernel calls

*Table C.3: Kernel calls.*

| File | Contents | Chapter |
|---|---|---|
| `kernel/src/ker/ability.rs` | abilities | 17 |
| `kernel/src/ker/channel.rs` | channels and connections | 10 |
| `kernel/src/ker/clock.rs` | `ClockTime()`, `ClockAdjust()`, CPU-time clocks | 13 |
| `kernel/src/ker/debug.rs` | stopping processes, breakpoints, single steps | 36 |
| `kernel/src/ker/dupfd.rs` | passing file descriptors (`ConnectDupFd()`) | 10 |
| `kernel/src/ker/entropy.rs` | `SysSrandom()` and interrupt entropy | 7 |
| `kernel/src/ker/errno.rs` | error numbers | B |
| `kernel/src/ker/event.rs` | events and their delivery | 11 |
| `kernel/src/ker/interrupt.rs` | interrupts for drivers | 15 |
| `kernel/src/ker/kpipe.rs` | kernel pipes | 12 |
| `kernel/src/ker/mod.rs` | `dispatch()`, `user_return()`, `unblock()`, scheduling calls, fork start-up | 7 |
| `kernel/src/ker/mqueue.rs` | message queues | 12 |
| `kernel/src/ker/msg.rs` | send, receive, reply, pulses | 10 |
| `kernel/src/ker/number.rs` | kernel-call numbers | A |
| `kernel/src/personality/mod.rs` | the personalities: what differs between a FreeBSD and a QNX process |
| `kernel/src/personality/native.rs` | the native personality (FreeBSD's ABI): its kernel calls `thr_new`, `thr_self`, the futex | 8 |
| `kernel/src/personality/qnx.rs` | the QNX personality: its error numbers, converted at the kernel boundary |
| `kernel/src/ker/offline.rs` | CPU offlining, `TimerDelegate()` | 8 |
| `kernel/src/ker/process.rs` | the process structure and table | 17 |
| `kernel/src/ker/servmon.rs` | the server monitor | 18 |
| `kernel/src/ker/signal.rs` | signals | 14 |
| `kernel/src/ker/sync.rs` | mutexes, condition variables, semaphores, barriers, read-write locks | 9 |
| `kernel/src/ker/thread.rs` | `ThreadCreate()`, join, cancel, `ThreadCtl()`, `SIGEV_THREAD` | 8 |
| `kernel/src/ker/timer.rs` | timers and `TimerTimeout()` | 13 |
| `kernel/src/ker/trace.rs` | kernel tracing | 35 |
| `kernel/src/ker/umem.rs` | access to user memory | 7 |

## Memory

*Table C.4: Memory.*

| File | Contents | Chapter |
|---|---|---|
| `kernel/src/vm/heap.rs` | the kernel heap | 16 |
| `kernel/src/vm/kmem.rs` | kernel pages, the linear map, frame windows | 16 |
| `kernel/src/vm/map.rs` | memory maps, `fork()` of memory, copy-on-write | 16 |
| `kernel/src/vm/mod.rs` | the memory module | 16 |
| `kernel/src/vm/object.rs` | memory objects | 16, 20 |
| `kernel/src/vm/pmem.rs` | page frames | 16 |

## Process 1

*Table C.5: Process 1.*

| File | Contents | Chapter |
|---|---|---|
| `kernel/src/proc/bootscript.rs` | the boot script | 19 |
| `kernel/src/proc/client.rs` | process 1 as a client of other servers | 18, 19 |
| `kernel/src/proc/debug.rs` | introspection messages | 36 |
| `kernel/src/proc/kinfo.rs` | `struct kinfo_proc`s, for `ps` and `top` | 18 |
| `kernel/src/proc/sysctl.rs` | `sysctl()`'s kernel variables | 18, 24 |
| `kernel/src/proc/devtext.rs` | the kernel console as a device | 22 |
| `kernel/src/proc/imagefs.rs` | the image filesystem | 22 |
| `kernel/src/proc/loader.rs` | the ELF loader and the initial stack | 19 |
| `kernel/src/proc/locks.rs` | record locks | 22 |
| `kernel/src/proc/memmgr.rs` | `mmap()` and the memory messages | 20 |
| `kernel/src/proc/mod.rs` | `proc::start()`: process 1's threads | 18 |
| `kernel/src/proc/namedsem.rs` | named semaphores | 32 |
| `kernel/src/proc/pathmgr.rs` | the path space and path resolution | 21 |
| `kernel/src/proc/procevent.rs` | process-manager events and watched values | 18 |
| `kernel/src/proc/procfs.rs` | `/proc` | 22 |
| `kernel/src/proc/procmgr.rs` | fork, wait, IDs, resources, cleanup of ended processes | 18 |
| `kernel/src/proc/resmgr.rs` | process 1's open files and I/O messages | 22 |
| `kernel/src/proc/rsrcdb.rs` | the resource database | — |
| `kernel/src/proc/session.rs` | sessions and controlling terminals | 18 |
| `kernel/src/proc/shmem.rs` | `/dev/shmem` objects | 20 |
| `kernel/src/proc/shmhandle.rs` | shared-memory handles | 32 |
| `kernel/src/proc/spawn.rs` | `spawn()` and `exec()` | 19 |
| `kernel/src/proc/sysmgr.rs` | the system-manager channel's loop | 18 |
| `kernel/src/proc/tty.rs` | the console's line discipline | 22 |
| `kernel/src/proc/tymem.rs` | typed memory | 20 |
| `kernel/src/proc/worker.rs` | the worker thread | 18 |

## Outside the kernel

*Table C.6: The rest of the tree.*

| Path | Contents | Chapter |
|---|---|---|
| `startup/` | the startup library and its board for QEMU's virt (it has a book of its own) | 5 |
| `user/proto/` | the layouts of kernel-call arguments and messages | 23 |
| `user/libsys/` | FreeBSD's system calls as kernel calls and messages | 24 |
| `user/kercall/`, `user/resmgr/` | the kernel calls and the server framework, for native servers | 25 |
| `user/pipe/` | the pipe server | 12, 25 |
| `user/pty/`, `user/random/`, `user/tmpfs/` | pseudo-terminals, the random device, the memory file system | 25 |
| `user/syslogd/`, `user/logring/` | the system logger and the log rings | 25 |
| `user/bsdkern/` | the kernel environment for FreeBSD's kernel code in a process | 27 |
| `user/netd/` | the network stack's server | 28 |
| `user/pcid/` | the PCI bus's server | 29 |
| `user/usbd/` | the USB server | 30 |
| `user/diskd/`, `user/bufcache/`, `user/md/`, `user/msdosfs/`, `user/fsd/` | disks and file systems | 31 |
| `user/qnxd/`, `user/qnxabi/` | the QNX translation service and its conversions | 32, 33 |
| `images/` | the buildfiles of the boot image | 3 |
| `tools/mkifs/`, `tools/qemu/`, `tools/test/` | the image builder, the QEMU launcher, the test runner | 3 |
| `tools/freebsd/`, `tools/sdk/` | FreeBSD's build scripts and patches; the SDK | 26, 27 |
| `tools/qnx/` | the probe that runs one program on both kernels | 34 |
| `tools/trace/`, `tools/kdump/` | collecting traces and crash dumps | 35, 36 |
| `tests/` | the test suite and its programs | 3, 34 |

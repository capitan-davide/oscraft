# Summary

[Preface](preface.md)

# Part I: Overview

- [Introduction](01-introduction.md)
- [The Architecture at a Glance](02-architecture.md)
- [Building, Running and Testing](03-build-run-test.md)

# Part II: The Microkernel

- [The Shape of the Kernel](04-kernel-shape.md)
- [Boot and the System Page](05-boot.md)
- [The AArch64 Layer](06-aarch64.md)
- [Kernel Calls](07-kernel-calls.md)
- [Threads and Scheduling](08-threads-scheduling.md)
- [Synchronization Objects](09-synchronization.md)
- [Message Passing](10-message-passing.md)
- [Pulses and Events](11-events.md)
- [Message Queues and Kernel Pipes](12-mqueue-kpipe.md)
- [Clocks, Timers and Timeouts](13-clocks-timers.md)
- [Signals](14-signals.md)
- [Interrupts](15-interrupts.md)
- [Memory](16-memory.md)
- [Processes and Abilities](17-processes.md)

# Part III: Process 1

- [The Process Manager](18-process-manager.md)
- [Spawning and Loading Programs](19-spawn-loader.md)
- [The Memory Manager](20-memory-manager.md)
- [The Pathname Manager](21-pathname-manager.md)
- [Built-in Resource Managers](22-builtin-resmgrs.md)

# Part IV: The Native System

- [The FreeBSD ABI in the Kernel](23-native-abi.md)
- [libsys: System Calls as Messages](24-libsys.md)
- [Resource Managers and the Native Servers](25-native-servers.md)
- [The Userland and the SDK](26-userland-sdk.md)

# Part V: FreeBSD's Kernel Code in Servers

- [bsdkern: a Kernel Inside a Process](27-bsdkern.md)
- [The Network Stack](28-network.md)
- [PCI](29-pci.md)
- [USB](30-usb.md)
- [Disks and File Systems](31-disks.md)

# Part VI: QNX Compatibility

- [The QNX Personality](32-qnx-personality.md)
- [qnxd and the Overlay](33-qnxd-overlay.md)
- [Measuring Against procnto](34-measuring-procnto.md)

# Part VII: Observing the System

- [Tracing](35-tracing.md)
- [Debugging and Crash Dumps](36-debugging.md)

---

[Appendix A: Kernel Call Numbers](appendix-a-kernel-calls.md)

[Appendix B: Error Numbers](appendix-b-errno.md)

[Appendix C: Source Map](appendix-c-source-map.md)

[Appendix D: Glossary](appendix-d-glossary.md)

# Appendix B: Source Map

Every source file of the crate, and the chapter that explains it.

*Table B.1: The source files.*

| File | Contents | Chapter |
|---|---|---|
| `src/lib.rs` | The crate root: `entry!`, `crash!`, the panic handler | 5 |
| `src/startup.rs` | The `Startup` context and its operations, `boot()` | 5, 6, 8, 14 |
| `src/global.rs` | `Global<T>`, startup's lock-free globals | 5 |
| `src/bootargs.rs` | The boot-arguments area mkifs fills in | 3 |
| `src/cmdline.rs` | Parsing the boot arguments; `getopt()`-style options | 5 |
| `src/options.rs` | The common options | 5 |
| `src/console.rs` | The debug UARTs, `-D`, `kprintln!` | 5 |
| `src/pl011.rs` | Arm's PL011 UART as a debug device | 5 |
| `src/ram.rs` | Range sets and RAM bookkeeping | 6 |
| `src/syspage/mod.rs` | The system page builder, layout and writing | 7 |
| `src/syspage/asinfo.rs` | The asinfo tree | 6, 7 |
| `src/syspage/hwinfo.rs` | The hwinfo tree and its tags | 7 |
| `src/syspage/strings.rs` | The strings and typed strings sections | 7 |
| `src/callout.rs` | The callout framework: `callout!`, patching, output | 8 |
| `src/qtime.rs` | The qtime section and the timer period | 10 |
| `src/elf.rs` | Planning the loading of ELF segments | 12 |
| `src/ifs.rs` | The image file system; loading the bootstrap executables | 12 |
| `src/decompress.rs` | Decompressing a UCL-compressed image file system | 3 |
| `src/testdata/ucl.bin` | Blocks of an image file system mkifs compressed, for the decompressor's test | 16 |
| `src/fdt.rs` | A flattened device tree reader | 15 |
| `src/abi/mod.rs` | The ABI types' module | 3, 7 |
| `src/abi/startup.rs` | The startup header and boot arguments | 3 |
| `src/abi/syspage.rs` | The system page and its sections | 7 |
| `src/abi/image.rs` | The image file system and bootstrap structures | 3, 12 |
| `src/abi/elf.rs` | ELF64 headers | 12 |
| `src/abi/layout_tests.rs` | The ABI types checked against `abi/layout.txt` | 16 |
| `src/arch/mod.rs` | The architectures | 1 |
| `src/arch/aarch64/mod.rs` | AArch64: per-CPU preparation | 4 |
| `src/arch/aarch64/entry.rs` | `_start`, the move to EL1 | 4 |
| `src/arch/aarch64/cache.rs` | Cleaning caches by set/way | 4 |
| `src/arch/aarch64/exceptions.rs` | Startup's exception vectors | 4 |
| `src/arch/aarch64/cpu.rs` | cpuinfo, cacheattr, ID registers, the timer | 10 |
| `src/arch/aarch64/gic_v2.rs` | The GICv2 and its callouts | 9 |
| `src/arch/aarch64/intr.rs` | Each CPU's part of the interrupt controller's setup | 9 |
| `src/arch/aarch64/mmu.rs` | The kernel's page tables | 11 |
| `src/arch/aarch64/smp.rs` | Starting the secondary CPUs, `smp_spin` | 13 |
| `src/arch/aarch64/handoff.rs` | Turning the MMU on and jumping | 14 |
| `abi/layout.c` | The C layouts, as array sizes | 16 |
| `abi/layout.py` | Reading the layouts into `abi/layout.txt` | 16 |
| `abi/layout.txt` | The layouts of QNX's headers | 16 |
| `boards/qemu-virt/Cargo.toml` | The reference board's crate: a static library | 15 |
| `boards/qemu-virt/src/lib.rs` | QEMU's `virt`: what its board crate is | 2, 15 |
| `boards/qemu-virt/src/board.rs` | `virt`'s `main()`: its memory map, debug UART, RAM and CPUs from the device tree | 2, 15 |
| `boards/qemu-virt/src/psci.rs` | PSCI through `hvc`: starting a CPU, the reboot callout | 2, 13, 15 |

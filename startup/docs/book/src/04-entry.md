# Entering Startup

This chapter covers startup's first instructions: the state the boot
loader leaves the CPU in, the move to the exception level QNX runs at,
the caches, and the safety net for startup's own bugs. It starts with the
AArch64 notions they rely on.

## Exception levels

An AArch64 CPU runs at one of four *exception levels*, EL0 to EL3, each
more privileged than the one below:

- **EL0** runs applications;
- **EL1** runs an operating system kernel;
- **EL2** runs a hypervisor, which can host several EL1 kernels;
- **EL3** runs the secure monitor, firmware that switches between the
  normal and the secure world.

A CPU starts at its highest level and only moves down by *returning*
from an exception: the code at the higher level writes the target's
state into the *Saved Program Status Register* (`SPSR_ELx`) and the
address to continue at into the *Exception Link Register* (`ELR_ELx`),
and executes `eret`. Each level also has registers that control what
the levels below may do: the *Hypervisor Configuration Register*
(`HCR_EL2`) decides, for instance, whether EL1 runs in AArch64 or AArch32,
and which of its operations trap to EL2.

QNX's kernel runs at EL1, and its processes at EL0. A boot loader may
enter startup at EL3 (an emulator with no firmware and `secure=on`), EL2
(most firmware, and U-Boot) or EL1 (a hypervisor's guest, and QEMU's
`virt` as OsCraft runs it). Startup brings
every CPU to EL1, with nothing trapping to EL2: the floating-point unit,
the timers and the ID registers all belong to EL1.

![Exception levels](images/ch04-levels.svg)

*Figure 4.1: The exception levels, where boot loaders enter startup, and where startup goes: EL1.*

## Running with the MMU off

Startup runs with the MMU off, as the boot loader leaves it. Two
consequences shape its code:

- **Addresses are physical.** A pointer to a device's register is the
  register's physical address; so is a pointer to the startup header.
  This is what lets startup build the kernel's page tables in plain
  memory without mapping anything first.
- **Every data access is to Device memory**: uncached and with strict
  ordering, and an unaligned access faults. The Rust target startup is
  built for, `aarch64-unknown-none-softfloat`, has `strict-align`, so the
  compiler never emits an unaligned access; and *softfloat* keeps it from
  using the floating-point registers, which trap until the kernel enables
  them.

Running uncached is slow, but startup does little work, and it avoids
a whole class of problems: nothing startup writes can sit in a cache when
the kernel turns the caches on.

## `_start`

Listing 4.1 is the entry point. It masks interrupts, saves `x0`-`x3`
(the boot loader's arguments; on `virt`, `x0` is the device tree's
address), moves to EL1, sets up a stack, finishes the CPU's
preparation in Rust, and calls the board's `main()` through the library
(Chapter 5).

*Listing 4.1: The entry point, `_start` (`src/arch/aarch64/entry.rs`).*

```rust
{{#include ../../../src/arch/aarch64/entry.rs:start}}
```

`adrp` and `:lo12:` form a symbol's address relative to the program
counter, which works wherever mkifs has placed startup.

## Down to EL1

Listing 4.2 moves the CPU down. It uses no stack and only a few
scratch registers, because the secondary CPUs run it too, before they
have a stack (Chapter 13).

*Listing 4.2: From EL3 or EL2 to EL1, `__oscraft_to_el1` (`src/arch/aarch64/entry.rs`).*

```rust
{{#include ../../../src/arch/aarch64/entry.rs:to_el1}}
```

At **EL3**, `SCR_EL3` makes the lower levels non-secure and AArch64 and
allows the `hvc` instruction; the CPU returns to EL2. On a CPU with
pointer authentication (`ID_AA64ISAR1_EL1`, `ID_AA64ISAR2_EL1` say), its
`API` and `APK` bits also keep the pointer authentication instructions
from trapping to EL3, and `HCR_EL2`'s same bits do so for EL2, so that
the kernel can use them.

At **EL2**, the code first turns the EL2 MMU and caches off: U-Boot
leaves them on. Then it prepares EL1:

- `HCR_EL2.RW` makes EL1 AArch64; `HCR_EL2.HCD` disables `hvc`, since
  nothing will run at EL2;
- `CPTR_EL2 = 0` and `HSTR_EL2 = 0`: no trap to EL2 for the
  floating-point unit or for system registers;
- `CNTHCTL_EL2 = 3` lets EL1 use the physical timer and counter, and
  `CNTVOFF_EL2 = 0` makes the virtual counter, which QNX's `ClockCycles()`
  reads, equal to the physical one;
- `VPIDR_EL2` and `VMPIDR_EL2` take the CPU's real ID and affinity, which
  EL1 reads as `MIDR_EL1` and `MPIDR_EL1`;
- `SCTLR_EL1` starts with its reserved-one bits only: MMU and caches off.

Then `eret` lands at EL1, where the code selects the EL1 stack pointer
(`SPSel = 1`) and returns.

> [!WARNING]
> `__oscraft_to_el1` returns to its caller at a lower exception level,
> with the same `x30`. It must not use the stack, which may not exist,
> and must leave `x0`-`x8` alone: callers keep values there. It uses
> `x9`-`x13`, and keeps its return address in `x13` while it calls the
> pointer authentication check.

## Clean caches

With the MMU off, startup's own accesses do not use the data cache. But
the cache may hold lines left by the boot loader, which ran with its caches
on. When the kernel turns the caches on, a stale line for memory startup
wrote would hide startup's data. So each CPU cleans and invalidates its
data caches once, before it writes anything the kernel reads.

The clean is *by set/way*: rather than naming addresses, it walks every
line of every cache level, using the cache's geometry from the
`CLIDR_EL1` and `CCSIDR_EL1` ID registers.

*Listing 4.3: Cleaning the caches by set/way, `clean_invalidate_dcache()` (`src/arch/aarch64/cache.rs`).*

```rust
{{#include ../../../src/arch/aarch64/cache.rs:clean_invalidate_dcache}}
```

For each level up to the *level of coherence* (`LoC`, the level beyond
which all observers see the same memory), the loop takes the line size,
number of ways and number of sets, and issues `dc cisw` (clean and
invalidate by set/way) for every combination. The way number goes in the
top bits of the operand, so its shift depends on how many ways there are.

## Exception vectors

Startup takes no interrupts, so any exception means a bug: a bad pointer,
an access that faults with the MMU off. Without a vector table, the CPU
would jump to whatever `VBAR_EL1` holds and hang. The library installs
vectors that report the exception on the debug console and stop.

*Listing 4.4: Startup's exception vectors (`src/arch/aarch64/exceptions.rs`).*

```rust
{{#include ../../../src/arch/aarch64/exceptions.rs:vectors}}
```

The table has 16 entries of 128 bytes, for four kinds of exception (a
synchronous one, IRQ, FIQ, SError) from four origins (the current level
with `SP_EL0` or `SP_ELx`, a lower level in AArch64 or AArch32). Each entry
loads its number and branches to common code, which collects the
*syndrome* (`ESR_EL1`: what happened), the faulting instruction's address
(`ELR_EL1`) and the faulting data address (`FAR_EL1`), and prints them.
A report looks like this:

```text
startup: unexpected synchronous exception: ESR_EL1=0x96000000 (class 0x25)
ELR_EL1=0x82d98 FAR_EL1=0xdead00000000 SPSR_EL1=0x600003c5 SP=0x960a0
```

Exception class `0x25` is a data abort at the current level; `FAR_EL1` is
the address that faulted, and `ELR_EL1` the instruction, which a
disassembly of startup (`objdump -d` of the linked image's startup) finds.

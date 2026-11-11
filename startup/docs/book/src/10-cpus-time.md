# CPUs and Time

The kernel needs to know about the CPUs it schedules on: what they are,
what features they have, how their caches are built. And it needs a clock.
This chapter covers the sections that describe both: `qtime`, `cpuinfo`,
`cacheattr`, and AArch64's `idreg_dump`.

## The generic timer

Every Armv8-A CPU has the same timer, the *generic timer*, in two parts:

- a **system counter**, shared by all CPUs, that counts up at a fixed
  frequency from power-on. Its frequency is in `CNTFRQ_EL0` (set by the
  firmware); the CPUs read it through `CNTPCT_EL0` (physical) or
  `CNTVCT_EL0` (virtual: the physical count less an offset that EL2 sets,
  `CNTVOFF_EL2`);
- per-CPU **timers** that compare the count against a value and raise an
  interrupt, a PPI: the virtual timer is PPI 27 on most boards.

QNX uses the virtual count for `ClockCycles()`, which user code calls, so
EL0 must be allowed to read it (`CNTKCTL_EL1.EL0VCTEN`), and the virtual
timer for the system tick. Startup sets the virtual offset to 0 (Chapter
4), allows the read, and stops the virtual timer until the kernel
programs it (`cpu::init_timer()`).

## `qtime`

The `qtime` section (Listing 10.1) gives the kernel the counter's frequency
and the tick's interrupt.

![Time on virt](images/ch10-time.svg)

*Figure 10.1: Time on `virt`: the counter, the timer that makes the tick, and what startup writes in `qtime` and `cpuinfo`.*

*Listing 10.1: The clock section, `QtimeEntry` (`src/abi/syspage.rs`).*

```rust
{{#include ../../../src/abi/syspage.rs:qtime_entry}}
```

The board's frequency can be replaced with `-f`: its third field is the
timer's frequency (and its first the CPUs' speed, for cpuinfo).

Besides the frequency, the kernel wants the counter's period as a decimal
fraction: `timer_period * 10^timer_scale` seconds. Listing 10.2 works it
out by long division of 1 by the frequency, one decimal digit at a time,
until the digits run out or no more fit in 32 bits.

*Listing 10.2: The counter's period, `invert_timer_freq()` (`src/qtime.rs`).*

```rust
{{#include ../../../src/qtime.rs:invert_timer_freq}}
```

`virt`'s counter runs at 62.5 MHz, a period of 16 ns: 16 × 10⁻⁹. A
counter at 54 MHz, a common rate on real boards, has a period of 18.518...
ns, which the division writes as 1851851851 × 10⁻¹⁷.

`timer_load_max` is the largest value the timer can be programmed with:
the generic timer's compare value is 64 bits. The flag
`QTIME_FLAG_GLOBAL_CLOCKCYCLES` says that `ClockCycles()` is the same on
every CPU, which is true of the shared system counter; the QNX startup
specification requires it.

## `cpuinfo`

Each CPU has a `cpuinfo` entry: its identification, its speed, its
features as flags, and its first-level caches. Two ID registers identify
it:

- `MIDR_EL1`: the implementer (0x41 for Arm), the part number (0xd08 for a
  Cortex-A72) and the revision;
- `MPIDR_EL1`: its *affinity*, its place in the system's hierarchy of
  clusters and cores. The kernel keeps it as `smp_hwcoreid`.

The flags say what the kernel and the C library may use:
the FPU and SIMD (from `ID_AA64PFR0_EL1`), the atomic and crypto
instructions (from `ID_AA64ISAR0_EL1`), whether the instruction cache needs
maintenance (`CTR_EL0.DIC`). Some flags are ARMv7's, kept for AArch32
programs, which test them.

The kernel needs both the FPU and the MMU on every CPU, so `CPU_FLAG_MMU`
is always set, and the library expects an FPU.

Some flags depend on options, which `Startup::init_boot_cpu()` passes on
as `Features`, the same for every CPU: SVE and pointer
authentication are flagged only if asked for (`-e sve`, `-U`) and the CPU
has them; `-E spectrev2` turns on the kernel's Spectre variant 2
mitigation (the MMU-based one on the Cortex-A57 and A72, the others'
otherwise), `-E ssbs` speculative store bypass safety; `-F` sets or clears
flags by hand, after everything else.

Listing 10.3 fills in a CPU's entry. It must run on that CPU, since ID
registers can only be read locally: the boot CPU runs it early, each
secondary CPU when it starts (Chapter 13).

*Listing 10.3: Describing a CPU, `cpu::init_cpuinfo()` (`src/arch/aarch64/cpu.rs`).*

```rust
{{#include ../../../src/arch/aarch64/cpu.rs:init_cpuinfo}}
```

## `cacheattr`

The kernel cleans and invalidates caches when it maps memory for DMA or
loads code. `cacheattr` lists the caches, each with its geometry and a
`control` callout that does the work on a range of addresses; a CPU's
`ins_cache` and `data_cache` index the list.

AArch64's cache maintenance by address (`dc cvac`, `dc civac`, `ic ivau`)
acts on every level up to the point of coherency, so the kernel only needs
the first level. The geometry comes from the ID registers: `CLIDR_EL1` says
which kinds of cache each level has, and `CCSIDR_EL1`, for the level and
kind `CSSELR_EL1` selects, gives the line size, ways and sets. CPUs with
the same caches share entries.

With `-X cachesnooped`, the data caches are flagged as kept coherent with
DMA by the hardware, so the kernel skips their maintenance for DMA
buffers.

The callouts (Listing 10.4) take a virtual address, a number of lines,
flags and the cache's entry. They do at most 32 lines per call, so that a
large request does not keep interrupts off for long, and return how many
they did; the instruction cache callout invalidates the whole cache for
very large requests and returns 0.

*Listing 10.4: The cache callouts (`src/arch/aarch64/cpu.rs`).*

```rust
{{#include ../../../src/arch/aarch64/cpu.rs:cache_callouts}}
```

## `idreg_dump`

The AArch64 part of the system page keeps a copy of each CPU's ID
registers, so that software can learn every CPU's features without running
on it: 56 registers per CPU, the whole ID register space `S3_0_C0_C1_0` to
`S3_0_C0_C7_7`. The library reads them all, with assembler loops; the
unallocated encodings in that space read as zero.

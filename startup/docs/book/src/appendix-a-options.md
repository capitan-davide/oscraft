# Appendix A: Startup Options

Startup's options come from the build file's bootstrap section, after the
startup program's name. The library takes the options of QNX's startup
library, with their syntax, so that existing build files work. A board
adds its own letters (a watchdog's timeout, say); `virt`'s adds none.

Sizes and addresses are numbers in C's notation (`0x1000`, `4096`,
`010`); sizes may end in `k`, `m` or `g`. Chapter 5 describes how the
options are parsed and when each takes effect.

*Table A.1: The options.*

| Option | Meaning |
|---|---|
| `-v` | More output on the debug console; repeat for more (`-vvv` lists the RAM given to the kernel) |
| `-D name[.base^shift.baud.clk]` | The debug UART, by the name the board gives it, for startup and the kernel; the options change its port (empty fields keep the board's defaults; the base is hexadecimal) |
| `-K name[.options]` | A second debug channel for the kernel, the same way |
| `-P n` | Start at most `n` CPUs |
| `-N name` | The host name (`_CS_HOSTNAME`); `localhost` by default |
| `-u reg` | The device tree's address is in `x0` at entry (any value but `arg` means the same) |
| `-u arg` | The device tree's address is in `argv[1]`, in hexadecimal: the boot loader called startup as `main(argc, argv)`, as U-Boot's `go` does |
| `-M addr,size` | More RAM |
| `-r addr,size[,1]` | Keep that RAM from the kernel and from startup's allocations; cleared, unless the third field is 1 |
| `-R size[,align[,name[,low]]]` | Set RAM aside under `name` (`reserved` by default), from the top of RAM, or the bottom if `low` is not 0 |
| `-j addr` | Store the system page's physical address at `addr`, for a JTAG debugger |
| `-c name:mask[,...]` | CPU clusters for the scheduler: the CPUs in each, as a bit mask |
| `-f cpu[,cycles[,timer]]` | Frequencies, instead of the board's: the CPUs' speed and the timer's (`cycles` is the timer's on AArch64); Hz, with `k` or `m`, or MHz if below 1000 |
| `-A` | Tell the kernel the last reboot was abnormal |
| `-Z` | The kernel runs without a periodic tick |
| `-o loop,overhead` | Nanospin calibration (the time of 100 loops, the call overhead), as a hwinfo device for the C library |
| `-E [~]meltdown,[~]spectrev2,[~]ssbs` | CPU errata: non-global mappings against Meltdown; the Spectre variant 2 mitigation; speculative store bypass safety (`~` turns one off) |
| `-e sve` | Let the kernel use SVE, if the CPUs have it |
| `-U` | Let the kernel use pointer authentication, if the CPUs have it |
| `-X cachesnooped` | The data caches are kept coherent with DMA by the hardware |
| `-X pagesize=1000` | 4 KB pages (the only size supported) |
| `-F [~]flags` | Set (or, with `~`, clear) cpuinfo flags on every CPU |

`-C`, `-H`, `-S` and `-T` are accepted and change nothing. `-C` makes
QNX's startup clear all the RAM it allocates, which this library always
does; `-H` matters only to a PSCI reboot callout, and `-T` to a boot time
read from a clock, which this library does not have; `-S` does nothing in
QNX's startup either.

*Table A.2: QNX's options this library does not support.*

| Option | In QNX's startup | Here |
|---|---|---|
| `-Q enable[,el1-host\|el2-host]` | Run the kernel as QNX Hypervisor's host, at EL2 | Reported and ignored: the kernel always runs at EL1 |
| `-I`, `-i` | Restore an image file system kept in RAM across a warm reset, and a second, non-bootable one | Reported and ignored |
| `-X pagesize=10000` | 64 KB pages | Reported and ignored |

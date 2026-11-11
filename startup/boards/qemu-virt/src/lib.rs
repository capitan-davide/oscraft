//! Startup for QEMU's virt machine, OsCraft's reference board.
//!
//! virt is a machine of QEMU's own, not a model of a real board: every fact
//! that can change with QEMU's command line (the RAM's size, the number of
//! CPUs) is in the device tree QEMU makes and passes in `x0`; the rest is
//! virt's fixed memory map (QEMU's `hw/arm/virt.c`). The board's part of
//! startup is what is left once the library has done the generic work:
//! where RAM and the GIC are, how to start a CPU and reset the machine
//! (PSCI), and the debug UART.

#![cfg_attr(not(test), no_std)]

#[cfg(target_arch = "aarch64")]
mod board;
#[cfg(target_arch = "aarch64")]
mod psci;

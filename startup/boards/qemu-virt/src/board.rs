//! The board's `main()`.

use crate::psci;
use oscraft_startup::arch::aarch64::gic_v2;
use oscraft_startup::console::{DebugDriver, DebugPort};
use oscraft_startup::fdt::{Fdt, cells};
use oscraft_startup::pl011;
use oscraft_startup::syspage::strings;
use oscraft_startup::{Startup, kprintln};

// ANCHOR: memory_map
/// virt's fixed memory map (QEMU's `hw/arm/virt.c`, `base_memmap`): the
/// GICv2's distributor and CPU interface, and RAM's start. The RAM's size
/// and the CPUs are the device tree's.
const GICD: u64 = 0x0800_0000;
const GICC: u64 = 0x0801_0000;
const RAM_BASE: u64 = 0x4000_0000;
/// The virtual timer's PPI (11, so interrupt 16 + 11): the system tick.
const TIMER_PPI: i32 = 27;
// ANCHOR_END: memory_map

// ANCHOR: debug_drivers
/// The debug UART: the PL011 at 0x09000000, from virt's 24 MHz APB clock
/// (the device tree's `apb-pclk`), QEMU's first serial port.
static DEBUG_DRIVERS: [DebugDriver; 1] = [pl011::driver(
    "pl011",
    DebugPort {
        base: 0x0900_0000,
        shift: 0,
        baud: 115_200,
        clk: 24_000_000,
    },
)];
// ANCHOR_END: debug_drivers

// ANCHOR: from_fdt
/// What the device tree says of the machine: RAM's size (`/memory`'s
/// `reg`, two cells each for address and size) and the number of CPUs
/// (`/cpus/cpu@n` nodes).
fn from_fdt(s: &Startup) -> (u64, usize) {
    let mut ram = 0;
    let mut cpus = 0;
    let Some((addr, _)) = s.fdt else {
        return (128 << 20, 1);
    };
    // SAFETY: the blob the boot loader passed, which set_fdt() checked;
    // startup runs with the MMU off, and nothing changes the blob.
    if let Some(fdt) = unsafe { Fdt::at(addr) } {
        for (node, _depth) in fdt.nodes() {
            if fdt.property(node, "device_type") == Some(b"memory\0")
                && let Some(reg) = fdt.property(node, "reg")
            {
                let c: [u32; 4] = core::array::from_fn(|i| cells(reg).nth(i).unwrap_or(0));
                ram += (u64::from(c[2]) << 32) | u64::from(c[3]);
            }
            if fdt.property(node, "device_type") == Some(b"cpu\0") {
                cpus += 1;
            }
        }
    }
    (ram, cpus.max(1))
}
// ANCHOR_END: from_fdt

// ANCHOR: board_main
fn main(s: &mut Startup) -> ! {
    s.init_debug(&DEBUG_DRIVERS);
    for opt in s.options("") {
        s.handle_common_option(&opt);
    }
    let (ram, num_cpus) = from_fdt(s);
    if s.debug_level > 0 {
        kprintln!("QEMU virt: {num_cpus} CPUs, {} MB of RAM", ram >> 20);
    }
    s.init_smp(num_cpus, psci::release_cpu);
    s.add_ram(RAM_BASE, ram);
    s.init_mmu();
    gic_v2::init(s, GICD, GICC);
    s.init_qtime(TIMER_PPI);
    // No clock-frequency in virt's tree: the CPUs' speed is unknown.
    s.init_boot_cpu(0);
    let sp = &mut s.syspage;
    sp.callout.reboot = sp.callout(&psci::REBOOT, &[]);
    sp.add_typed_string(strings::CS_MACHINE, "QEMU virt");
    sp.add_typed_string(strings::CS_HW_PROVIDER, "QEMU");
    s.boot()
}
// ANCHOR_END: board_main

oscraft_startup::entry!(main);

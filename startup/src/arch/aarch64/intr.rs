//! The interrupt controller's part of each CPU's setup.
//!
//! Every CPU, the boot CPU and each secondary one as it starts, prepares
//! its own view of the interrupt controller (with a GIC, its CPU
//! interface) and records its `gic_map` entry, the target the kernel
//! passes to the `send_ipi` callout. What that takes depends on the
//! controller, so the controller's code registers it here: [`gic_v2`]'s
//! `init()` does, and a board with another controller registers its own
//! with [`crate::Startup::set_cpu_intr_init`].
//!
//! [`gic_v2`]: super::gic_v2

use crate::global::Global;

/// Set up the interrupt controller for CPU `cpu`, on that CPU, and record
/// its `gic_map` entry. `num_cpu` is the number of CPUs in the system.
pub type CpuIntrInit = fn(gic_map: &mut [u64], cpu: usize, num_cpu: u16);

static CPU_INIT: Global<Option<CpuIntrInit>> = Global::new(None);

/// Register `init` as the controller's per-CPU setup. Called by the boot
/// CPU, before any other CPU starts.
pub fn set_cpu_init(init: CpuIntrInit) {
    // SAFETY: only the boot CPU runs, and nothing else holds CPU_INIT.
    unsafe { *CPU_INIT.get() = Some(init) };
}

/// Run the registered per-CPU setup for CPU `cpu`.
pub fn init_cpu(gic_map: &mut [u64], cpu: usize, num_cpu: u16) {
    // SAFETY: written by the boot CPU before any CPU calls this.
    let Some(init) = (unsafe { *CPU_INIT.get() }) else {
        crate::crash!("no interrupt controller set up");
    };
    init(gic_map, cpu, num_cpu);
}

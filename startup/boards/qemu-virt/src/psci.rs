//! PSCI, Arm's Power State Coordination Interface: firmware calls that
//! start and stop CPUs and reset or power off the machine. On virt, QEMU
//! itself implements them, behind `hvc` (the device tree's
//! `/psci/method`), as a hypervisor would: startup runs at EL1, and the
//! call traps to QEMU.

// ANCHOR: psci_calls
/// `CPU_ON` (the SMC64 function): start the CPU whose MPIDR is the first
/// argument at the second, with the third in its `x0`.
const CPU_ON: u64 = 0xc400_0003;

/// Start CPU `cpu` (its MPIDR affinity: virt numbers them 0, 1, ...) at
/// `entry`, in the library's secondary entry code, at EL1 with its MMU off.
pub fn release_cpu(cpu: usize, entry: u64) {
    let ret: i64;
    // SAFETY: a PSCI call to QEMU; it starts another CPU and returns.
    unsafe {
        core::arch::asm!(
            "hvc #0",
            inout("x0") CPU_ON => ret,
            in("x1") cpu as u64,
            in("x2") entry,
            in("x3") 0u64,
        );
    }
    if ret != 0 {
        oscraft_startup::kprintln!("psci: CPU_ON {cpu}: error {ret}");
    }
}

// The kernel's reboot callout: PSCI's SYSTEM_RESET (0x84000009), which
// QEMU answers by resetting the machine. No patches: virt has nothing to
// map for it.
oscraft_startup::callout!(pub static REBOOT, "psci_reboot_hvc", patches = 0,
    "movz    x0, #0x0009",
    "movk    x0, #0x8400, lsl #16",
    "hvc     #0",
    "0: b    0b",
);
// ANCHOR_END: psci_calls

//! Interprocessor interrupts: one CPU asks another to reschedule (a thread
//! it should run became ready), or to look at its timers again.
//!
//! As in QNX, through the system page: startup's `smp.send_ipi` callout
//! ORs the command into the target CPU's command word and, if none was
//! pending, raises the interrupt that `intrinfo` marks
//! `INTR_CONFIG_FLAG_IPI` on that CPU (`interrupt::ipi_vector`); the
//! target takes its commands in the handler. The kernel knows no
//! interrupt controller.

use core::sync::atomic::Ordering;

use crate::percpu;
use crate::syspage::SyspageEntry;

/// Run the scheduler: a thread this CPU should run is ready.
pub const RESCHED: u32 = 1;
/// The CPU's family of timers changed, or a timer of it was armed
/// elsewhere (`clock::family_changed`).
pub const TIMERS: u32 = 2;

type SendIpi = unsafe extern "C" fn(*mut SyspageEntry, u64, u32, *mut u32);

/// Take this CPU's interprocessor interrupts (once per CPU, after
/// `interrupt::init`). Without an IPI vector or callout, CPUs notice work
/// at their next clock tick.
pub fn init_cpu() {
    let Some(v0) = crate::interrupt::ipi_vector() else {
        return;
    };
    if crate::syspage::get().smp().send_ipi == 0 {
        return;
    }
    if let Some(v) = crate::interrupt::local_vector(v0) {
        let _ = crate::interrupt::attach(v, handler);
    }
}

/// Send `cmd` to CPU `cpu` (not the caller).
pub fn send(cpu: u32, cmd: u32) {
    let sp = crate::syspage::get();
    let callout = sp.smp().send_ipi;
    let Some(target) = percpu::get(cpu) else {
        return;
    };
    if callout == 0 || crate::interrupt::ipi_vector().is_none() {
        return;
    }
    // SAFETY: the system page's address of startup's C-ABI callout.
    let send: SendIpi = unsafe { core::mem::transmute(callout) };
    // SAFETY: the callout takes the system page, the target CPU, the
    // command and the target's command word.
    unsafe { send(sp.as_ptr(), cpu as u64, cmd, target.ipi_cmds.as_ptr()) };
}

fn handler(_vector: u32) {
    let me = percpu::this();
    let cmds = me.ipi_cmds.swap(0, Ordering::AcqRel);
    if cmds & RESCHED != 0 {
        crate::sched::request_resched();
    }
    if cmds & TIMERS != 0 {
        crate::clock::family_changed();
    }
}

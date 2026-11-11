//! Per-CPU data, reached through the architecture's per-CPU register
//! (`TPIDR_EL1` on AArch64).

use core::sync::atomic::{AtomicBool, AtomicU64};

use crate::arch;

pub const MAX_CPUS: usize = 8;

#[derive(Debug)]
pub struct PerCpu {
    /// Logical CPU number, as numbered by startup (cpuinfo index).
    pub cpu: u32,
    pub online: AtomicBool,
    /// Clock ticks taken on this CPU.
    pub ticks: AtomicU64,
    /// Kernel calls made on this CPU.
    pub kercalls: AtomicU64,
    /// Counter value of this CPU's next tick.
    pub deadline: AtomicU64,
    /// While this CPU idles: the counter value of the tick it wakes at for
    /// the timers it looks after (`u64::MAX`: none due); 0 while it ticks
    /// (`clock::stop_tick`).
    pub idle_until: AtomicU64,
    pub sched: crate::sched::CpuSched,
    /// Two kernel pages for copying user memory (`ker::umem`).
    pub copy_window: AtomicU64,
    /// The frames the two copy windows map now (`u64::MAX`: none).
    pub copy_window_pa: [AtomicU64; 2],
    /// Two kernel pages for reaching page frames (`vm::kmem::with_frame`).
    pub frame_window: AtomicU64,
    /// The frames the two frame windows map now (`u64::MAX`: none).
    pub frame_window_pa: [AtomicU64; 2],
    /// The TTBR0_EL1 value loaded (`arch::uspace::activate`).
    pub user_ttbr0: AtomicU64,
    /// Counter value of this CPU's next high-resolution timer expiry
    /// (`u64::MAX`: none), see `clock`.
    pub hrt_deadline: AtomicU64,
    /// Interprocessor-interrupt commands pending for this CPU (`ipi`).
    pub ipi_cmds: core::sync::atomic::AtomicU32,
    /// The priority of the thread this CPU runs (0: idle), for the CPU
    /// that wakes a thread to choose whom to interrupt (`sched::wake`).
    pub running_prio: core::sync::atomic::AtomicU8,
    /// An IPI asked this CPU to reschedule and it has not yet: a second
    /// wakeup goes to another idle CPU (`sched::kick_for`).
    pub kicked: AtomicBool,
    /// Handling an interrupt (`interrupt::handle`): work that may fault
    /// pages in or allocate is left to a kernel thread.
    pub in_irq: AtomicBool,
    /// `SCTLR_EL1.A` is set: alignment checks for the EL0 code of a thread
    /// that asked for them (`arch::exception::set_align_check`).
    pub align_check: AtomicBool,
    /// Taken offline (`SchedCtl(SCHED_PROCESSOR_OFFLINE)`): only
    /// `SCHED_OFFLINING` threads run here, and no IPI comes (`ker::offline`).
    pub offline: AtomicBool,
    /// The CPUs whose timers this CPU looks after (bit n: CPU n's): its
    /// own unless delegated (`TimerDelegate()`), and others' delegated to
    /// it (`ker::offline`, `clock`).
    pub timer_family: AtomicU64,
    /// The clock's work came while this CPU ran a thread at or above the
    /// clock's priority, and waits for it to drop (`clock`).
    pub clock_held: AtomicBool,
}

impl PerCpu {
    const fn new(cpu: u32) -> Self {
        Self {
            cpu,
            online: AtomicBool::new(false),
            ticks: AtomicU64::new(0),
            kercalls: AtomicU64::new(0),
            deadline: AtomicU64::new(0),
            idle_until: AtomicU64::new(0),
            sched: crate::sched::CpuSched::new(),
            copy_window: AtomicU64::new(0),
            copy_window_pa: [const { AtomicU64::new(u64::MAX) }; 2],
            frame_window: AtomicU64::new(0),
            frame_window_pa: [const { AtomicU64::new(u64::MAX) }; 2],
            user_ttbr0: AtomicU64::new(0),
            hrt_deadline: AtomicU64::new(u64::MAX),
            ipi_cmds: core::sync::atomic::AtomicU32::new(0),
            running_prio: core::sync::atomic::AtomicU8::new(0),
            kicked: AtomicBool::new(false),
            align_check: AtomicBool::new(false),
            in_irq: AtomicBool::new(false),
            offline: AtomicBool::new(false),
            timer_family: AtomicU64::new(1 << cpu),
            clock_held: AtomicBool::new(false),
        }
    }
}

static CPUS: [PerCpu; MAX_CPUS] = {
    let mut cpus = [const { PerCpu::new(0) }; MAX_CPUS];
    let mut i = 0;
    while i < MAX_CPUS {
        cpus[i] = PerCpu::new(i as u32);
        i += 1;
    }
    cpus
};

/// Bind the calling CPU to its per-CPU data. Called first on every CPU.
pub fn init_cpu(cpu: u32) {
    assert!((cpu as usize) < MAX_CPUS, "CPU {cpu} beyond MAX_CPUS");
    arch::set_percpu(&CPUS[cpu as usize] as *const PerCpu as u64);
}

/// The calling CPU's data.
pub fn this() -> &'static PerCpu {
    // SAFETY: `init_cpu` stored a pointer into CPUS on this CPU.
    unsafe { &*(arch::percpu() as *const PerCpu) }
}

pub fn get(cpu: u32) -> Option<&'static PerCpu> {
    CPUS.get(cpu as usize)
}

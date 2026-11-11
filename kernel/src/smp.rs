//! Starting the other CPUs.
//!
//! Startup parks CPUs 1..n in its `smp_spin` callout with the MMU on. Each
//! store of an entry address to `smp.start_addr` releases exactly one of
//! them (it swaps the value back to 0), which jumps there with x0 = system
//! page and x1 = its CPU number (BSP `lib/aarch64/callout_smp_spin.S`).

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch;
use crate::arch::mmu::Prot;
use crate::percpu;
use crate::syspage::Syspage;
use crate::vm::kmem;
use crate::vm::pmem::PAGE_SIZE;

/// Kernel stack size of a CPU's boot thread.
const STACK_PAGES: u64 = 16;

/// Stack for the CPU being released; read by `ap_start` in boot.s.
#[unsafe(no_mangle)]
static AP_STACK_TOP: AtomicU64 = AtomicU64::new(0);

/// CPUs released but numbered past `percpu::MAX_CPUS` (bit n - MAX_CPUS:
/// CPU n), left parked (`refuse`), and how many.
static REFUSED: AtomicU64 = AtomicU64::new(0);
static REFUSED_COUNT: AtomicU64 = AtomicU64::new(0);

/// A released CPU numbered past `percpu::MAX_CPUS`: it has no per-CPU
/// data, so it can't take a console lock or anything else. It records
/// itself and waits forever, interrupts masked; CPU 0 reports it (review CORE-24).
pub fn refuse(cpu: u32) -> ! {
    let bit = (cpu as usize).saturating_sub(percpu::MAX_CPUS);
    if bit < 64 {
        REFUSED.fetch_or(1 << bit, Ordering::SeqCst);
    }
    REFUSED_COUNT.fetch_add(1, Ordering::SeqCst);
    loop {
        arch::wait_for_interrupt();
    }
}

unsafe extern "C" {
    fn ap_start();
}

/// Release CPUs 1..num_cpu one at a time; returns how many came online.
/// Any parked CPU may take a release, so we wait for the number of online
/// CPUs (or of refused ones, numbered past `MAX_CPUS`) to grow rather than
/// for a particular one, and release every parked CPU: a refused one used
/// up a release that another, in range, still needs.
pub fn start_secondaries(sp: &Syspage) -> u32 {
    let num_cpu = sp.header().num_cpu as u32;
    let hz = arch::timer::frequency();
    let online = || {
        (1..percpu::MAX_CPUS as u32)
            .filter(|&c| percpu::get(c).unwrap().online.load(Ordering::Acquire))
            .count() as u32
            + REFUSED_COUNT.load(Ordering::SeqCst) as u32
    };
    let mut started = 0;
    while started + 1 < num_cpu {
        let Some(stack) = kmem::map_new_guarded(STACK_PAGES, Prot::Data) else {
            crate::kprintln!("smp: no memory for a CPU stack");
            break;
        };
        AP_STACK_TOP.store(stack + STACK_PAGES * PAGE_SIZE, Ordering::SeqCst);
        let smp = sp.smp_ptr();
        #[expect(
            clippy::multiple_unsafe_ops_per_block,
            reason = "the `(&raw mut (*smp).start_addr)` projection and its volatile write are one access to one field"
        )]
        // SAFETY: a field of the system page's smp section, which is
        // writable by the kernel; a spinning CPU takes it with an exclusive
        // swap and resets it to 0.
        unsafe {
            (&raw mut (*smp).start_addr).write_volatile(ap_start as *const () as usize)
        };
        // SAFETY: a barrier and an event only: the waiting CPUs wake.
        unsafe { core::arch::asm!("dsb ish", "sev", options(nostack)) };
        let deadline = arch::timer::counter() + hz;
        while online() == started && arch::timer::counter() < deadline {
            core::hint::spin_loop();
        }
        if online() == started {
            crate::kprintln!("smp: no CPU came online within 1 s");
            break;
        }
        started += 1;
    }
    let refused = REFUSED.load(Ordering::SeqCst);
    if REFUSED_COUNT.load(Ordering::SeqCst) != 0 {
        crate::kprintln!(
            "smp: CPUs numbered from {} on left parked (MAX_CPUS): mask {refused:#x} from CPU {}",
            percpu::MAX_CPUS,
            percpu::MAX_CPUS
        );
    }
    started - REFUSED_COUNT.load(Ordering::SeqCst) as u32
}

/// CPUs online, including CPU 0.
pub fn online_count() -> u32 {
    online_mask().count_ones()
}

/// The CPUs online as a bit mask (bit n: CPU n), including CPU 0.
pub fn online_mask() -> u64 {
    (1..percpu::MAX_CPUS as u32)
        .filter(|&c| percpu::get(c).unwrap().online.load(Ordering::Acquire))
        .fold(1, |m, c| m | 1 << c)
}

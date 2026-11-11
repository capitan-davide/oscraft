//! OsCraft microkernel.

#![no_std]
#![no_main]

extern crate alloc;

mod arch;
mod bootargs;
mod clock;
mod console;
mod msgbuf;
mod interrupt;
mod ipi;
mod kdebug;
mod ker;
mod percpu;
mod personality;
mod proc;
mod report;
mod sched;
mod selftest;
mod smp;
mod sporadic;
mod sync;
mod syspage;
mod timeout;
mod vm;

use core::panic::PanicInfo;

use syspage::{Syspage, SyspageEntry};

/// Called by `boot.s` on CPU 0, in EL1 with startup's MMU setup.
///
/// # Safety
///
/// `syspage` must be the system page passed by startup.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmain(syspage: *mut SyspageEntry, cpu: u32) -> ! {
    // SAFETY: guaranteed by the caller (startup, via boot.s).
    let syspage = unsafe { Syspage::new(syspage) };
    percpu::init_cpu(cpu);
    syspage::set(syspage);
    // The address programs see the system page at, chosen as procnto
    // chooses it (`loader::adopt_user_syspage`).
    proc::loader::adopt_user_syspage(syspage);
    console::init(syspage);

    let h = syspage.header();
    if h.type_ != syspage::SYSPAGE_AARCH64 {
        panic!("system page type {:#x} is not AArch64", h.type_);
    }
    if let Err(e) = syspage.check() {
        panic!("system page: {e}");
    }
    kprintln!("OsCraft kernel {} on CPU {cpu}", env!("CARGO_PKG_VERSION"));
    kprintln!(
        "system page at {:p}: type {:#x}, version {}.{}, {} bytes, {} CPUs",
        syspage.as_ptr(),
        h.type_,
        h.version.major,
        h.version.minor,
        h.total_size,
        h.num_cpu
    );
    report::print(&syspage);

    arch::exception::init();
    let ok = arch::exception::selftest();
    kprintln!(
        "exceptions: vectors at {:#x}, self-test {}",
        arch::exception::vector_base(),
        if ok { "ok" } else { "FAILED" }
    );

    let (ranges, bytes) = vm::pmem::init(&syspage);
    kprintln!("pmem: {} MiB free in {ranges} ranges", bytes >> 20);

    let mmu = arch::mmu::init();
    kprintln!(
        "mmu: adopted startup's kernel tables, TCR {:#x} (T1SZ {}, identity map off), MAIR {:#x}",
        mmu.tcr,
        mmu.t1sz,
        mmu.mair
    );
    match arch::mmu::selftest() {
        Ok((va, pa)) => {
            kprintln!("mmu: self-test ok (mapped {va:#x} -> {pa:#x}, new L2/L3 tables)")
        }
        Err(e) => panic!("mmu self-test: {e}"),
    }
    let selftests = bootargs::BootArgs::get().is_some_and(|b| b.selftests());
    if selftests {
        selftest::heap();
    }
    vm::pmem::init_free_list();
    let linear = vm::kmem::init_linear(&syspage);
    kprintln!("kmem: {} MiB of RAM in the linear map", linear >> 20);
    // procnto -mc (OsCraft's): copy-on-write fork() instead of QNX 8's copy.
    let cow = bootargs::BootArgs::get().and_then(|b| b.memmgr('c')) == Some(true);
    vm::map::set_cow_fork(cow);
    kprintln!(
        "vm: fork() {}",
        if cow { "shares pages copy-on-write (-mc)" } else { "copies pages" }
    );
    // procnto -U: servers may deliver unregistered events (QNX 8 allows only
    // registered ones by default).
    let unregistered = bootargs::BootArgs::get().is_some_and(|b| b.flag("-U"));
    ker::event::set_unregistered_events(unregistered);
    if unregistered {
        kprintln!("events: unregistered events allowed (-U)");
    }
    ker::process::init();
    arch::uspace::init_cpu(ker::process::KERNEL.aspace().ttbr0());
    ker::umem::init_cpu();
    vm::kmem::init_cpu();
    if selftests {
        selftest::uspace();
    }

    match interrupt::init(&syspage) {
        Ok(i) => kprintln!(
            "interrupts: {} controllers from the system page ({} cascaded, not supported yet)",
            i.controllers,
            i.cascaded_skipped
        ),
        Err(e) => panic!("interrupts: {e}"),
    }

    // procnto -C tick_frequency[,clock_IST_priority].
    let (hz, ist_prio) = bootargs::BootArgs::get().map_or(Ok((1000, 254)), |b| b.clock()).unwrap_or_else(|e| {
        kprintln!("kernel: {e}; the defaults are used");
        (1000, 254)
    });
    let c = clock::init(&syspage, hz, ist_prio).unwrap_or_else(|e| panic!("clock: {e}"));
    kprintln!(
        "clock: {} Hz tick on vector {} ({} Hz counter), clock priority {}",
        clock::tick_hz(),
        c.vector,
        c.counter_hz,
        clock::ist_priority()
    );

    sched::init_cpu();
    ker::debug::init_cpu();
    arch::context::fp_enable_cpu();
    ipi::init_cpu();
    kdebug::init();
    // CPU 0 takes IPIs from now on: a thread woken elsewhere that should
    // run here comes with one (`sched::kick_for`).
    percpu::this().online.store(true, core::sync::atomic::Ordering::Release);
    let started = smp::start_secondaries(&syspage);
    kprintln!("smp: {} of {} CPUs online", started + 1, h.num_cpu);

    arch::irq_enable();
    // OsCraft's self-tests, only if asked for (`OSCRAFT_SELFTEST=1`): a
    // second of ticks counted on each CPU, then the scheduler, sleeping,
    // kernel calls, messages and user processes. The boot script waits for
    // their end (`bootscript::START`). Without them the boot script starts
    // at once, as on procnto (review CORE-18).
    if selftests {
        let start = arch::timer::counter();
        while clock::ticks(0) < clock::tick_hz() {
            arch::wait_for_interrupt();
        }
        let elapsed_ms = (arch::timer::counter() - start) * 1000 / c.counter_hz;
        kprint!("clock: after {elapsed_ms} ms, ticks per CPU:");
        for cpu in 0..=started {
            kprint!(" {}", clock::ticks(cpu));
        }
        kprintln!(", {} spurious interrupts", interrupt::spurious_count());
        selftest::sched();
        selftest::sleep();
    } else {
        proc::bootscript::START.store(true, core::sync::atomic::Ordering::Release);
    }
    proc::start(&syspage);
    sched::idle()
}

/// Entry of CPUs 1..n (from `ap_start` in boot.s), on their own stack.
///
/// # Safety
///
/// `syspage` must be the system page; called once per CPU by startup's
/// smp_spin callout via `smp::start_secondaries`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kmain_ap(syspage: *mut SyspageEntry, cpu: u32) -> ! {
    if cpu as usize >= percpu::MAX_CPUS {
        smp::refuse(cpu);
    }
    percpu::init_cpu(cpu);
    arch::exception::init();
    arch::mmu::init_cpu();
    arch::uspace::init_cpu(ker::process::KERNEL.aspace().ttbr0());
    ker::umem::init_cpu();
    vm::kmem::init_cpu();
    sched::init_cpu();
    ker::debug::init_cpu();
    arch::context::fp_enable_cpu();
    clock::start_cpu().unwrap_or_else(|e| panic!("cpu{cpu}: clock: {e}"));
    ipi::init_cpu();
    let _ = syspage;
    kprintln!("cpu{cpu}: online");
    percpu::this()
        .online
        .store(true, core::sync::atomic::Ordering::Release);
    arch::irq_enable();
    sched::idle()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    kprintln!("kernel panic: {info}");
    kdebug::crash(None, kdebug::sigcode(kdebug::SIGABRT, 0, 0));
    arch::halt()
}

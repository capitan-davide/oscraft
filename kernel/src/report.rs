//! Boot report: what the kernel learnt from startup, printed on the console.

use crate::bootargs::BootArgs;
use crate::kprintln;
use crate::syspage::{CACHE_LIST_END, CPU_FLAG_FPU, CPU_FLAG_MMU, Syspage};

pub fn print(sp: &Syspage) {
    match BootArgs::get() {
        Some(args) => {
            kprintln!("command line:");
            kprint_list("  argv:", args.argv());
            kprint_list("  env: ", args.env());
            kprintln!("  startup header at {:#x}", args.shdr_addr);
        }
        None => kprintln!("command line: none"),
    }

    let p = sp.system_private();
    kprintln!(
        "system private: page size {}, syspage kern {:#x} user {:#x}, cpupage kern {:#x} user {:#x} spacing {}",
        p.pagesize,
        p.kern_syspageptr,
        p.user_syspageptr,
        p.kern_cpupageptr,
        p.user_cpupageptr,
        p.cpupage_spacing
    );

    kprintln!("cpus:");
    for c in sp.cpuinfo() {
        kprintln!(
            "  {:#010x} {} MHz hwid {:#x} \"{}\" flags {:#x}{}{} caches i:{} d:{}",
            c.cpu,
            c.speed,
            c.smp_hwcoreid,
            sp.string(c.name),
            c.flags,
            if c.flags & CPU_FLAG_MMU != 0 {
                " mmu"
            } else {
                ""
            },
            if c.flags & CPU_FLAG_FPU != 0 {
                " fpu"
            } else {
                ""
            },
            c.ins_cache,
            c.data_cache
        );
    }

    kprintln!("caches:");
    for (i, c) in sp.cacheattr().enumerate() {
        crate::kprint!(
            "  {i}: flags {:#x} {} lines x {} bytes, {} ways",
            c.flags,
            c.num_lines,
            c.line_size,
            c.ways
        );
        if c.next != CACHE_LIST_END {
            crate::kprint!(", next {}", c.next);
        }
        kprintln!();
    }

    let q = sp.qtime();
    kprintln!(
        "timer: {} cycles/s, interrupt {}, period {}e{} s, load max {:#x}, flags {:#x}",
        q.cycles_per_sec,
        q.intr,
        q.timer_period,
        q.timer_scale,
        q.timer_load_max,
        q.flags
    );

    kprintln!("interrupt controllers:");
    for i in sp.intrinfo() {
        kprintln!(
            "  vectors {:#x}..{:#x} cascade {:#x} flags {:#x} local stride {:#x}",
            i.vector_base,
            i.vector_base + i.num_vectors,
            i.cascade_vector,
            i.flags,
            i.local_stride
        );
    }

    kprintln!("physical address space:");
    for a in sp.asinfo() {
        let mut depth = 0;
        let mut owner = sp.asinfo_index(a.owner);
        while let Some(i) = owner {
            depth += 1;
            owner = sp.asinfo().nth(i).and_then(|e| sp.asinfo_index(e.owner));
        }
        kprintln!(
            "  {:indent$}{:#012x}-{:#012x} {} (attr {:#x})",
            "",
            a.start,
            a.end,
            sp.string(a.name),
            a.attr,
            indent = depth * 2
        );
    }

    let c = sp.callout();
    kprintln!(
        "callouts: reboot {:?}, debug0 display_char {:?}, smp start_addr {:#x}",
        c.reboot,
        c.debug[0].display_char,
        sp.smp().start_addr
    );
}

fn kprint_list(label: &str, items: impl Iterator<Item = &'static str>) {
    crate::kprint!("{label}");
    for s in items {
        crate::kprint!(" {s}");
    }
    kprintln!();
}

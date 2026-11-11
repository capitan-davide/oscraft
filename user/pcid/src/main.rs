//! `pcid`: FreeBSD's PCI bus as a server. It finds the host bridges from
//! the board's device tree (the system page's), enumerates the buses,
//! assigns the devices' windows and interrupts, and owns configuration
//! space: drivers in other servers reach their devices' through it, and
//! map their windows and attach their interrupts themselves. See
//! `docs/book`.
//!
//! `pcid [-v] [-e <name>=<value>]... [-m <module>]...`: `-e` adds to
//! FreeBSD's kernel environment, as the loader's (`hint.pcib.0.disabled=1`);
//! `-m` loads a module, a board's host bridge from its BSP, whose driver
//! attaches to the device tree's node (QEMU's virt needs none: its generic
//! ECAM bridge is built in).

#[cfg(target_os = "freebsd")]
fn main() {
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut it = args.iter();
    let mut modules = Vec::new();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-v" => bsdkern::env::verbose(),
            "-m" => modules.extend(it.next().cloned()),
            "-e" => {
                let e = it.next().map_or("", |s| s.as_str());
                if !bsdkern::env::kenv(e) {
                    eprintln!("pcid: -e {e}: not <name>=<value>");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("usage: pcid [-v] [-e <name>=<value>]... [-m <module>]...");
                std::process::exit(1);
            }
        }
    }
    bsdkern::start(cpus);
    for m in &modules {
        if let Err(e) = bsdkern::env::kldload(m) {
            eprintln!("pcid: {m}: error {e}");
            std::process::exit(1);
        }
    }
    // `/dev/pci` (pciconf(8), and the drivers' configuration space).
    bsdkern::devfs::serve(2);
    println!("pcid: FreeBSD's PCI bus is up on {cpus} CPUs");
    loop {
        std::thread::park();
    }
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}

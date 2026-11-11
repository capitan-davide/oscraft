//! `usbd`: FreeBSD's USB core and host controllers as a server; class
//! drivers live in the servers that use them (netd's `if_cdce`). See
//! `docs/book`.
//!
//! `usbd [-v] [-e <name>=<value>]... [-o <sysctl>=<value>]... [pci] [<controller>=<address>,<irq>[,<size>]]...`:
//! `-e` adds to FreeBSD's kernel environment (`hw.bsdkern.dma_pool=below1G`,
//! the typed-memory pool for DMA, as `io-usb-otg -t memory=`);
//! `pci` takes the host controllers on the PCI bus (xhci), through pcid,
//! which owns the bus (bsdkern's PCI client bus); the others name a host controller
//! (`dwcotg`, the DesignWare one) with its registers at physical
//! `<address>` (`<size>` bytes, 64 KB by default) and its interrupt, as
//! QNX's drivers take theirs on the command line: the kernel and usbd
//! know nothing of the board. `-o` sets an integer sysctl of FreeBSD's USB
//! code (`hw.usb.dwc_otg.debug=1`), before the controllers attach.

#[cfg(target_os = "freebsd")]
unsafe extern "C" {
    fn bsdkern_nexus_add(name: *const core::ffi::c_char, mem: u64, size: u64, irq: i32) -> i32;
    fn usbd_sysctl_int(name: *const core::ffi::c_char, value: i32) -> i32;
    fn bsdkern_pciclient_start() -> i32;
}

#[cfg(target_os = "freebsd")]
fn number(s: &str) -> Option<u64> {
    match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

#[cfg(target_os = "freebsd")]
fn main() {
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-v") {
        bsdkern::env::verbose();
    }
    // The kernel environment, before anything reads it.
    for w in args.windows(2).filter(|w| w[0] == "-e") {
        if !bsdkern::env::kenv(&w[1]) {
            eprintln!("usbd: -e {}: not <name>=<value>", w[1]);
            std::process::exit(1);
        }
    }
    bsdkern::start(cpus);
    println!("usbd: FreeBSD's USB core is up on {cpus} CPUs");
    let mut args = args.iter().filter(|a| *a != "-v");
    while let Some(a) = args.next() {
        if a == "-e" {
            args.next();
            continue;
        }
        if a == "-o" {
            let o = args.next().map_or("", |s| s.as_str());
            let Some((name, value)) = o.split_once('=').and_then(|(n, v)| Some((n, v.parse::<i32>().ok()?))) else {
                eprintln!("usbd: -o {o}: not <sysctl>=<integer>");
                std::process::exit(1);
            };
            let cname = std::ffi::CString::new(name).unwrap_or_default();
            // SAFETY: a NUL-terminated name.
            let r = unsafe { usbd_sysctl_int(cname.as_ptr(), value) };
            if r != 0 {
                eprintln!("usbd: -o {o}: error {r}");
            }
            continue;
        }
        if a == "pci" {
            // SAFETY: bsdkern's call, once FreeBSD's code is up.
            let r = unsafe { bsdkern_pciclient_start() };
            if r != 0 {
                eprintln!("usbd: pci: error {r}");
                std::process::exit(1);
            }
            continue;
        }
        let parsed = a.split_once('=').and_then(|(name, rest)| {
            let v: Vec<u64> = rest.split(',').map(number).collect::<Option<_>>()?;
            match v[..] {
                [mem, irq] => Some((name, mem, 0x10000, irq)),
                [mem, irq, size] => Some((name, mem, size, irq)),
                _ => None,
            }
        });
        let Some((name, mem, size, irq)) = parsed else {
            eprintln!("usbd: {a}: not <controller>=<address>,<irq>[,<size>]");
            std::process::exit(1);
        };
        let cname = std::ffi::CString::new(name).unwrap_or_default();
        // SAFETY: a NUL-terminated name; the rest are numbers.
        let r = unsafe { bsdkern_nexus_add(cname.as_ptr(), mem, size, irq as i32) };
        if r != 0 {
            eprintln!("usbd: {name} at {mem:#x}, irq {irq}: error {r}");
            std::process::exit(1);
        }
    }
    // `/dev/usbd` (the class drivers' servers) and the core's own nodes;
    // a thread each for transfers that wait.
    bsdkern::devfs::serve(16);
    loop {
        std::thread::park();
    }
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}

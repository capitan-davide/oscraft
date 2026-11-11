//! `tmpfs`: file systems in memory. `tmpfs [-s megabytes] [-n nodes]
//! [-m mode] [-u uid] [-g gid] path...`: each path its own, empty, with
//! the options given before it (`-m` its root's permissions, 0755 unless
//! given; `-s` and `-n` its limits, none unless given).

#[cfg(target_os = "freebsd")]
fn main() {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    let (mut bytes, mut nodes, mut mode, mut uid, mut gid) = (0u64, 0u64, 0o755u32, 0u32, 0u32);
    let mut args = std::env::args().skip(1);
    let mut served = 0;
    let usage = || -> ! {
        eprintln!("usage: tmpfs [-s megabytes] [-n nodes] [-m mode] [-u uid] [-g gid] path...");
        std::process::exit(2)
    };
    while let Some(a) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "-s" => bytes = value().parse::<u64>().unwrap_or_else(|_| usage()) << 20,
            "-n" => nodes = value().parse().unwrap_or_else(|_| usage()),
            "-m" => mode = u32::from_str_radix(&value(), 8).unwrap_or_else(|_| usage()),
            "-u" => uid = value().parse().unwrap_or_else(|_| usage()),
            "-g" => gid = value().parse().unwrap_or_else(|_| usage()),
            p if !p.starts_with('-') => {
                let fs = tmpfs::Tmpfs::new(mode, uid, gid, bytes, nodes, now);
                if let Err(err) = fsd::attach(p, "tmpfs", Box::new(fs)) {
                    eprintln!("tmpfs: can't mount {p}: error {err}");
                    std::process::exit(1);
                }
                served += 1;
            }
            _ => usage(),
        }
    }
    if served == 0 {
        usage();
    }
    fsd::serve()
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}

//! `netd`: FreeBSD's network stack as a server. See `docs/book`.

#[cfg(target_os = "freebsd")]
mod rights;
#[cfg(target_os = "freebsd")]
mod selftest;
#[cfg(target_os = "freebsd")]
mod server;
#[cfg(target_os = "freebsd")]
mod unix;

#[cfg(target_os = "freebsd")]
unsafe extern "C" {
    fn bsdnet_loopback_up() -> core::ffi::c_int;
}

#[cfg(target_os = "freebsd")]
fn main() {
    let cpus = std::thread::available_parallelism().map_or(1, |n| n.get());
    if std::env::args().any(|a| a == "-v") {
        bsdkern::env::verbose();
    }
    bsdkern::start(cpus);
    if std::env::args().any(|a| a == "-t") {
        println!("netd: FreeBSD's kernel code is up on {cpus} CPUs");
    }
    bsdkern::env::check("lo0", {
        // SAFETY: once, after start-up.
        unsafe { bsdnet_loopback_up() }
    });
    if std::env::args().any(|a| a == "-t") {
        selftest::run();
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let prefix = args.iter().position(|a| a == "-p").and_then(|i| args.get(i + 1));
    server::run(prefix.map_or("", String::as_str));
}

#[cfg(not(target_os = "freebsd"))]
fn main() {}

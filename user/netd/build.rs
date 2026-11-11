//! Links `libbsdnet.a` (`make bsdnet`) whole: its SYSINITs are found through
//! linker sets, which nothing refers to by name.
use std::env;
use std::path::Path;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("freebsd") {
        return;
    }
    let root = env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib = Path::new(&root).join("../../build/freebsd/libbsdnet.a");
    println!("cargo:rerun-if-changed={}", lib.display());
    println!("cargo:rustc-link-arg-bins=-Wl,--whole-archive,{},--no-whole-archive", lib.display());
    println!("cargo:rustc-link-arg-bins=-Wl,--no-gc-sections");
}

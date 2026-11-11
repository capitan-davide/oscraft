//! The thread-local variables FreeBSD's headers read (`__thread`, which
//! stable Rust cannot define): compiled with the sysroot's C compiler.
use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=csrc/tls.c");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("freebsd") {
        return;
    }
    let cc = env::var("CARGO_TARGET_AARCH64_UNKNOWN_FREEBSD_LINKER").expect("the sysroot's compiler");
    let out = env::var("OUT_DIR").unwrap();
    let obj = Path::new(&out).join("tls.o");
    let lib = Path::new(&out).join("libbsdkern_tls.a");
    let run = |c: &mut Command| assert!(c.status().expect("run").success());
    run(Command::new(&cc).args(["-O2", "-c", "csrc/tls.c", "-o"]).arg(&obj));
    let _ = std::fs::remove_file(&lib);
    run(Command::new("llvm-ar").arg("rcs").arg(&lib).arg(&obj));
    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-lib=static=bsdkern_tls");
}

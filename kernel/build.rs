fn main() {
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    println!("cargo:rustc-link-arg-bins=-T{dir}/kernel.ld");
    println!("cargo:rerun-if-changed=kernel.ld");
    println!("cargo:rerun-if-changed=src/arch/aarch64/boot.s");
}

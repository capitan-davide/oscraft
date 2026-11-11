//! The SDK check: a Rust program built with the SDK's Cargo
//! configuration (std from source for aarch64-unknown-freebsd), run on
//! OsCraft.
use std::collections::BTreeMap;
use std::thread;

fn main() {
    let handle = thread::spawn(|| (1..=10u64).product::<u64>());
    let mut m = BTreeMap::new();
    m.insert("ten", handle.join().expect("a thread"));
    let text = format!("sdk-rust: hello, 10! = {}", m["ten"]);
    println!("{text}, args={}", std::env::args().count());
}

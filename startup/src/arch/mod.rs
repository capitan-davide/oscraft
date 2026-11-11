//! Architecture support. QNX 8.0 runs on AArch64 and x86_64; this crate
//! supports AArch64.

#[cfg(target_arch = "aarch64")]
pub mod aarch64;

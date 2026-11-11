//! `bsdkern`: what FreeBSD's kernel code expects of its kernel, for a
//! program on OsCraft that links that code in (`netd`, FreeBSD's network
//! stack from `libbsdnet.a`). QNX's `io-sock` is the same arrangement.
//! FreeBSD's upper layers stay: this supplies what lies under them.

#![cfg_attr(not(target_os = "freebsd"), allow(dead_code))]

#[cfg(target_os = "freebsd")]
pub use env::start;

#[cfg(target_os = "freebsd")]
pub mod devfs;

pub mod dma;
#[cfg(target_os = "freebsd")]
pub mod env;
#[cfg(target_os = "freebsd")]
pub mod lock;
#[cfg(target_os = "freebsd")]
pub mod mem;
#[cfg(target_os = "freebsd")]
pub mod park;
#[cfg(target_os = "freebsd")]
pub mod privilege;
#[cfg(target_os = "freebsd")]
pub mod slot;

pub mod syspage;
#[cfg(target_os = "freebsd")]
pub mod thread;

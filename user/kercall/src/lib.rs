//! OsCraft's kernel interface for native (FreeBSD-personality) programs
//! and servers, from oscraft-libc's platform layer: the kernel calls
//! (`neutrino`, QNX's `<sys/neutrino.h>` as C functions and Rust
//! functions), the connect protocol (`path`), the I/O messages a client
//! sends (`io`), and the layouts (`abi`). What oscraft-libc took from
//! its own C library, `errno` and the process ID, comes from FreeBSD's
//! libc here (`tls`).

#![no_std]

pub mod abi;
pub mod arch;
pub mod io;
pub mod mem;
pub mod neutrino;
pub mod path;
pub mod process;
pub mod time;
pub mod tls;

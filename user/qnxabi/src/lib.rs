//! What differs between QNX's and FreeBSD's ABIs: the conversions, as
//! pure functions, compiled into libsys, `qnxd` and the kernel. When to
//! convert is theirs to decide. It goes when the last QNX program does.

#![no_std]

pub mod dirent;
pub mod errno;
pub mod fcntl;
pub mod stat;
pub mod termios;

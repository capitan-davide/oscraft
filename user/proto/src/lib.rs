//! The system interface as data: the messages that clients and servers
//! exchange, the events they deliver, the information the kernel gives
//! about a client, and the numbers they all agree on (error numbers,
//! device commands, flags). Each type has QNX OS 8.0's layout, which a
//! check against QNX's headers confirms (`tools/check-proto.py`).
//!
//! The C library builds its requests from these types, and its resource
//! managers read them; OsCraft's own servers, and later its kernel, are
//! to share them. The crate holds no code that talks to the system: it
//! builds and tests anywhere.
//!
//! - [`pod`], for viewing a message as bytes;
//! - [`io`], for the I/O messages of `<sys/iomsg.h>`;
//! - [`ioctl`], for the `ioctl()` commands the library translates;
//! - [`connect`], for `_IO_CONNECT`, the path requests;
//! - [`event`], for `struct sigevent`;
//! - [`fcntl`], for the open flags and `fcntl()`'s commands;
//! - [`info`], for what the kernel says about a client or a message;
//! - [`proc`], for the process manager's messages;
//! - [`mem`], for the memory manager's;
//! - [`stat`], for `struct stat`;
//! - [`sys`], for the system manager's;
//! - [`dcmd`], for the device commands and their data;
//! - [`errno`] and [`fcntl`], the system's error numbers and open flags
//!   (FreeBSD's), and [`qnx_errno`] and [`qnx_fcntl`], QNX's;
//! - [`layout`], for the table the layout check compares.

#![no_std]

pub mod connect;
pub mod dcmd;
pub mod errno;
pub mod fcntl;
pub mod qnx_errno;
pub mod event;
pub mod qnx_fcntl;
pub mod info;
pub mod io;
pub mod ioctl;
pub mod layout;
pub mod mem;
pub mod pod;
pub mod proc;
pub mod rsrc;
pub mod stat;
pub mod sys;
pub mod termios;

/// `SYSMGR_COID` (`<sys/neutrino.h>`): every process's connection to
/// the process manager, which the memory and system managers share.
pub const SYSMGR_COID: i32 = 0x4000_0000;

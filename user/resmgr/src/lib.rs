//! The server layer of OsCraft's C library: QNX's framework for writing
//! servers, `<sys/dispatch.h>`, `<sys/resmgr.h>` and `<sys/iofunc.h>`.
//!
//! On QNX, a driver or a file system is an ordinary process that owns
//! paths and answers the messages clients send to them. This framework is
//! how almost every such server is written: it receives the messages,
//! keeps track of which client has which file open, and calls the
//! server's functions, with POSIX defaults for all it leaves out. It sits
//! above the POSIX layer, which its defaults use, and is QNX's own API,
//! not POSIX's or ISO C's.
//!
//! - [`dispatch`], for the channel, its handlers and the receive loop;
//! - [`resmgr`], for paths, open files and the I/O protocol;
//! - [`iofunc`], for the POSIX defaults;
//! - [`pool`], for thread pools;
//! - [`name`], for `name_attach()` and `name_open()`;
//! - [`abi`], for the public structures.

#![no_std]

pub mod abi;
pub mod acl;
pub mod dispatch;
pub mod iofunc;
pub mod mem;
pub mod name;
pub mod pool;
pub mod resmgr;
pub mod select;
pub mod compat;
